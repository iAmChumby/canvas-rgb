//! The session epoch order the stack preview and the stack draft share (D95.4.4).
//!
//! A page makes a random 16-hex-character epoch when the builder opens, and every PUT carries a
//! sequence that starts at 1 for that epoch. The daemon gives each epoch it first sees a
//! monotonically increasing generation, and the newest generation owns the route: a PUT from an
//! older epoch is superseded, whatever a Stop, an expiry, a layout switch or a DELETE did to the
//! slot in between, and however many pages have loaded. A page the daemon has never seen is a
//! new page and always wins, so a reload is never locked out.
//!
//! The book keeps one entry per epoch the daemon has seen and never evicts one. An entry is a
//! 16-character epoch and a generation, about 50 bytes; the builder opens an epoch when it opens
//! and a heavy day of reloads is a few hundred kilobytes for the lifetime of the daemon. A fixed
//! memory (a 32-entry list, say) is exactly the bug this replaces: once the owning epoch falls
//! out of it, the daemon can no longer tell a stale tab from a page that just opened, and the
//! stale tab takes the lights.
//!
//! The order is by when the daemon first saw a page, not by when the page loaded: a page whose
//! first PUT arrives after a newer page's takes the route, because the newest user action wins.
//! Only a page that has been superseded is stale.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// True for a session epoch: exactly 16 hexadecimal characters, as the page makes it.
pub fn valid_epoch(epoch: &str) -> bool {
    epoch.len() == 16 && epoch.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The epochs of one route (the preview, or the draft): which one owns it and which ones are
/// older. Every method costs one lock and a map lookup.
#[derive(Default)]
pub struct EpochBook {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The generation the next epoch the daemon has never seen gets.
    next: u64,
    /// Every epoch this daemon has seen, and the generation it was given.
    seen: BTreeMap<String, u64>,
    /// The newest generation issued. It only moves when a new epoch appears.
    ceiling: u64,
    /// The newest epoch the daemon has seen. It stays after the route is cleared, so a stale
    /// tab can be told which page owns the route even when nothing does.
    owner: Option<String>,
    /// The epoch and the highest sequence accepted from it, while one owns the route.
    live: Option<(String, u64)>,
}

impl EpochBook {
    /// Offers a PUT. `true` when it may apply: the epoch is the newest page, and its sequence is
    /// above the live one's (or it is a new page, which always wins). `false` when an older
    /// epoch, or a sequence the route already saw, is superseded.
    pub fn offer(&self, epoch: &str, seq: u64) -> bool {
        let mut book = self.lock();
        let generation = match book.seen.get(epoch) {
            Some(generation) => *generation,
            None => {
                book.next += 1;
                let generation = book.next;
                book.seen.insert(epoch.to_string(), generation);
                book.ceiling = generation;
                book.owner = Some(epoch.to_string());
                generation
            }
        };
        if generation < book.ceiling {
            // A tab the daemon saw before a newer page opened: it learns it lost.
            return false;
        }
        if let Some((live_epoch, live_seq)) = &book.live {
            if live_epoch == epoch && seq <= *live_seq {
                return false;
            }
        }
        book.live = Some((epoch.to_string(), seq));
        true
    }

    /// The epoch that owns the route and the highest sequence accepted from it, if one does.
    pub fn live(&self) -> Option<(String, u64)> {
        self.lock().live.clone()
    }

    /// The newest epoch the daemon has seen, even after Stop or DELETE cleared the live one.
    pub fn owner(&self) -> Option<String> {
        self.lock().owner.clone()
    }

    /// True when `epoch` owns the route now: it is the live epoch, or, when nothing is live
    /// because a Stop or a DELETE cleared it, the newest page the daemon has seen. A page that
    /// has been superseded never owns it, however many loads have happened since.
    pub fn holds(&self, epoch: &str) -> bool {
        let book = self.lock();
        match &book.live {
            Some((live, _)) => live == epoch,
            None => book.owner.as_deref() == Some(epoch),
        }
    }

    /// The route is empty (Stop, expiry, DELETE, a replacement). The generations stay: the tab
    /// that lost still lost, and the newest page can take the route again with its next PUT.
    pub fn clear_live(&self) {
        self.lock().live = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_epoch_is_sixteen_hex_characters() {
        assert!(valid_epoch("0123456789abcdef"));
        assert!(valid_epoch("ABCDEF0123456789"));
        assert!(!valid_epoch(""));
        assert!(!valid_epoch("0123456789abcde"));
        assert!(!valid_epoch("0123456789abcdef0"));
        assert!(!valid_epoch("0123456789abcdeg"));
        assert!(!valid_epoch("0123456789ABCDE-"));
    }

    #[test]
    fn the_first_epoch_owns_and_a_lower_sequence_is_refused() {
        let book = EpochBook::default();
        let first = "1111111111111111";
        assert!(book.offer(first, 1));
        assert!(book.offer(first, 2));
        assert!(!book.offer(first, 2));
        assert!(!book.offer(first, 1));
        assert_eq!(book.live(), Some((first.to_string(), 2)));
    }

    #[test]
    fn a_new_epoch_wins_and_an_older_one_stays_lost() {
        let book = EpochBook::default();
        let older = "1111111111111111";
        let newer = "2222222222222222";
        assert!(book.offer(older, 1));
        assert!(book.offer(newer, 1));
        assert!(!book.offer(older, 99), "an older tab never takes the route back");
        assert_eq!(book.live(), Some((newer.to_string(), 1)));
    }

    #[test]
    fn forty_page_loads_do_not_forget_the_oldest() {
        let book = EpochBook::default();
        let oldest = "0000000000000000";
        assert!(book.offer(oldest, 1));
        for page in 1..=40u64 {
            let epoch = format!("{page:016x}");
            assert!(book.offer(&epoch, 1), "a new page always wins");
        }
        assert!(!book.offer(oldest, 2), "the stale tab still loses after 40 loads");
    }

    #[test]
    fn clearing_the_route_keeps_the_order() {
        let book = EpochBook::default();
        let older = "1111111111111111";
        let newer = "2222222222222222";
        assert!(book.offer(older, 1));
        assert!(book.offer(newer, 1));
        // Stop, expiry, DELETE or a replacement: the slot is gone, not the order.
        book.clear_live();
        assert_eq!(book.live(), None);
        assert!(!book.offer(older, 2), "a stale tab does not re-take after a Stop");
        assert!(book.offer(newer, 2), "the newest page can take it again");
        assert_eq!(book.live(), Some((newer.to_string(), 2)));
    }

    #[test]
    fn a_page_the_daemon_has_never_seen_always_wins() {
        let book = EpochBook::default();
        assert!(book.offer("aaaaaaaaaaaaaaaa", 7));
        book.clear_live();
        assert!(book.offer("bbbbbbbbbbbbbbbb", 1));
    }
}
