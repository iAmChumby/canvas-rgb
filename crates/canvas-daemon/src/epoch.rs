//! The session epoch order the stack preview and the stack draft share (D95.4.4).
//!
//! A page makes a random 16-hex-character epoch when the builder opens, and every PUT carries a
//! sequence that starts at 1 for that epoch. The daemon gives each epoch it first sees a
//! monotonically increasing generation, and the newest generation owns the route: a PUT from an
//! older epoch is superseded, whatever a Stop, an expiry, a layout switch or a DELETE did to the
//! slot in between, and however many pages have loaded. A page the daemon has never seen is a
//! new page and always wins, so a reload is never locked out.
//!
//! The book keeps the epochs it has seen in two tiers, so its memory does not grow with the
//! number of page loads: the exact entries (the epoch and its generation) for the pages that are
//! still talking to the daemon, and a fingerprint of every epoch that has fallen out of them. A
//! page whose entry fell out is an older page, and it keeps losing: the fingerprint is enough to
//! know the daemon has seen it before, and it never takes the lights from a newer page. Only a
//! page the daemon has never seen at all is a new page.
//!
//! The order is by when the daemon first saw a page, not by when the page loaded: a page whose
//! first PUT arrives after a newer page's takes the route, because the newest user action wins.
//! Only a page that has been superseded is stale.

use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

/// How many epochs the book remembers exactly: the newest page and the pages that are still
/// talking to the daemon.
///
/// The cap is safe because the entry that can still win is never dropped. The owner (the newest
/// epoch the daemon has seen) is always in the book, so the newest page keeps the route and
/// every older page keeps losing to it. The entries that fall out are the ones that have been
/// superseded by this many other page loads, and the daemon still recognises them (see
/// [`EPOCH_BOOK_FORGOTTEN`]), so a page that falls out never takes the lights from a newer one.
/// The pages that are still talking are kept longer than the quiet ones: the book drops the
/// least recently used entry that can no longer win, so a stale tab that keeps sending beats
/// stays known and keeps losing.
pub const EPOCH_BOOK_CAPACITY: usize = 1024;

/// How many forgotten epochs the book still refuses, as a fingerprint each.
///
/// A fingerprint is eight bytes, so the daemon can remember a good many of them: a page that
/// fell out of the book keeps losing for thousands of page loads after it did.
pub const EPOCH_BOOK_FORGOTTEN: usize = 8192;

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
    /// Every epoch this daemon still remembers exactly, and the generation it was given.
    seen: BTreeMap<String, Entry>,
    /// The newest generation issued. It only moves when a new epoch appears.
    ceiling: u64,
    /// The newest epoch the daemon has seen. It stays after the route is cleared, so a stale
    /// tab can be told which page owns the route even when nothing does.
    owner: Option<String>,
    /// The epoch and the highest sequence accepted from it, while one owns the route.
    live: Option<(String, u64)>,
    /// The fingerprints of the epochs that fell out of `seen`. One of them goes when the set
    /// is full.
    forgotten: HashSet<u64>,
    /// The clock the book stamps its entries with. The least recently used entry has the
    /// smallest stamp.
    used: u64,
}

/// One epoch the book remembers: the generation it was given, and when the daemon last heard
/// from it.
struct Entry {
    generation: u64,
    used: u64,
}

impl Entry {
    /// The entry of an epoch the daemon saw, at the time it last heard from it.
    fn new(generation: u64, used: u64) -> Self {
        Entry { generation, used }
    }
}

impl EpochBook {
    /// Offers a PUT. `true` when it may apply: the epoch is the newest page, and its sequence is
    /// above the live one's (or it is a new page, which always wins). `false` when an older
    /// epoch, or a sequence the route already saw, is superseded.
    pub fn offer(&self, epoch: &str, seq: u64) -> bool {
        let mut book = self.lock();
        let stamp = book.tick();
        let known = book.seen.get(epoch).map(|entry| entry.generation);
        let generation = match known {
            Some(generation) => {
                // The daemon heard from this epoch again: it is the page it knows, and it is
                // the most recently used entry now.
                if let Some(entry) = book.seen.get_mut(epoch) {
                    entry.used = stamp;
                }
                generation
            }
            None => {
                // The book does not remember this epoch. It is either a page the daemon has
                // never seen (a new page, which always wins) or one it has forgotten (an older
                // page, which must never take the lights from a newer one).
                let fingerprint = fingerprint(epoch);
                if book.is_forgotten(fingerprint) {
                    // A page that fell out of the book: it is older than the newest page, and
                    // it learns it lost. Its fingerprint stays in the book, so a page that
                    // keeps talking after it fell out keeps losing.
                    return false;
                }
                book.next += 1;
                let generation = book.next;
                let entry = Entry::new(generation, stamp);
                book.seen.insert(epoch.to_string(), entry);
                book.ceiling = generation;
                book.owner = Some(epoch.to_string());
                book.trim();
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

    /// How many epochs the book remembers exactly. The tests watch it stay bounded.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().seen.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Inner {
    /// The next stamp for the book's least-recently-used order.
    fn tick(&mut self) -> u64 {
        self.used += 1;
        self.used
    }

    /// Keeps the book within [`EPOCH_BOOK_CAPACITY`].
    ///
    /// The entry that can still win (the owner, the only one whose generation is the ceiling)
    /// is never dropped. The entries that can no longer win go first, the least recently used of
    /// them first, so a tab that keeps sending beats stays known and keeps losing.
    fn trim(&mut self) {
        while self.seen.len() > EPOCH_BOOK_CAPACITY {
            let victim = self
                .seen
                .iter()
                .filter(|(_, entry)| entry.generation != self.ceiling)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(epoch, _)| epoch.clone());
            let Some(victim) = victim else {
                // Only the entry that can still win is left: it stays.
                return;
            };
            self.seen.remove(&victim);
            self.forget(&victim);
        }
    }

    /// Remembers a fingerprint of an epoch the book drops, so a page that comes back is refused
    /// rather than treated as a new page.
    fn forget(&mut self, epoch: &str) {
        self.forgotten.insert(fingerprint(epoch));
        while self.forgotten.len() > EPOCH_BOOK_FORGOTTEN {
            // Any fingerprint will do: the set holds thousands of them, so a page that fell
            // out keeps losing for thousands of page loads after it did.
            let Some(dropped) = self.forgotten.iter().next().copied() else {
                return;
            };
            self.forgotten.remove(&dropped);
        }
    }

    /// True when the daemon has seen this epoch and forgotten it: it is an older page, and it
    /// must never take the route from a newer one.
    fn is_forgotten(&self, fingerprint: u64) -> bool {
        self.forgotten.contains(&fingerprint)
    }
}

/// A small fingerprint of an epoch: eight bytes, so the daemon can remember many of them.
fn fingerprint(epoch: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    epoch.hash(&mut hasher);
    hasher.finish()
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

    #[test]
    fn the_book_keeps_at_most_its_capacity_of_epochs() {
        let book = EpochBook::default();
        let loads = EPOCH_BOOK_CAPACITY as u64 * 4;
        for page in 0..loads {
            let epoch = format!("{page:016x}");
            assert!(book.offer(&epoch, 1), "a new page always wins");
        }
        assert!(
            book.len() <= EPOCH_BOOK_CAPACITY,
            "the book holds {} entries after {loads} page loads",
            book.len()
        );
    }

    #[test]
    fn the_newest_pages_still_win_and_the_older_ones_still_lose_after_the_book_rolled_over() {
        let book = EpochBook::default();
        let oldest = "0000000000000000";
        assert!(book.offer(oldest, 1));
        let loads = EPOCH_BOOK_CAPACITY as u64 * 2;
        for page in 1..=loads {
            let epoch = format!("{page:016x}");
            assert!(book.offer(&epoch, 1), "a new page always wins");
        }
        let newest = format!("{:016x}", loads + 1);
        assert!(book.offer(&newest, 1), "the newest page owns the route");
        assert!(!book.offer(oldest, 2), "the oldest page still loses");
        assert!(
            !book.offer(&newest, 1),
            "a sequence the route already saw is refused"
        );
        assert!(book.offer(&newest, 2), "the newest page can send again");
    }

    #[test]
    fn a_page_that_fell_out_of_the_book_never_wins_over_a_newer_one() {
        let book = EpochBook::default();
        let stale = "1111111111111111";
        assert!(book.offer(stale, 1), "the page owns the route");
        let loads = EPOCH_BOOK_CAPACITY as u64 * 2;
        for page in 0..loads {
            let epoch = format!("{page:016x}");
            assert!(book.offer(&epoch, 1), "a new page always wins");
        }
        let newer = "2222222222222222";
        assert!(book.offer(newer, 1), "the newer page takes the route");
        assert!(
            !book.offer(stale, 2),
            "the page that fell out of the book never wins over a newer one"
        );
        assert_eq!(
            book.live(),
            Some((newer.to_string(), 1)),
            "the newer page kept the route"
        );
    }

    #[test]
    fn the_page_that_owns_the_route_is_never_dropped_from_the_book() {
        let book = EpochBook::default();
        let owner = "1111111111111111";
        assert!(book.offer(owner, 1), "the page owns the route");
        let loads = EPOCH_BOOK_CAPACITY as u64 * 3;
        for page in 0..loads {
            let epoch = format!("{page:016x}");
            assert!(book.offer(&epoch, 1), "a new page always wins");
        }
        assert!(
            !book.offer(owner, 2),
            "the page that owned the route keeps losing to the newer pages"
        );
        assert!(
            book.holds(&format!("{:016x}", loads - 1)),
            "the newest page owns the route"
        );
    }
}
