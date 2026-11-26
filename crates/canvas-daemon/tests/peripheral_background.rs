//! #567: the Attack Shark X6 is never read in the background. A background read lands on an idle
//! mouse, the person coming back wakes it mid-burst, and on 2026-11-26 that left the receiver
//! out of step until it was replugged. The X6 is read only by the person's **Read again**.
//!
//! The peripherals are the fakes-mode ones (`canvas_daemon::fakes::fake_peripherals`), which take
//! their answer to `background_reads` from the real adapters. No HID device is opened.

use canvas_daemon::fakes::fake_peripherals;
use canvas_daemon::peripherals::{InputIdle, Peripheral, PeripheralHub};
use canvas_proto::device::DeviceId;
use canvas_proto::peripheral::{PeripheralAccess, PeripheralKind};
use std::sync::Arc;
use std::time::Duration;

/// A person who has been away for a minute: every background read is allowed by the clock.
fn away() -> Arc<dyn InputIdle> {
    Arc::new(|| Duration::from_secs(60))
}

async fn id_of(hub: &PeripheralHub, kind: PeripheralKind) -> DeviceId {
    hub.discover()
        .await
        .into_iter()
        .find(|info| info.kind == kind)
        .map(|info| info.id)
        .unwrap_or_else(|| panic!("the fakes list a {kind:?}"))
}

#[tokio::test]
async fn an_idle_pass_reads_the_controller_and_never_the_mouse() {
    let hub = PeripheralHub::with_idle(fake_peripherals(), away());
    let mouse = id_of(&hub, PeripheralKind::Mouse).await;
    let controller = id_of(&hub, PeripheralKind::GameController).await;
    for id in [&mouse, &controller] {
        hub.apply(id, PeripheralAccess { reads: true, writes: true }).await.expect("the fake opens");
    }

    let read = hub.refresh_idle(&[mouse.clone(), controller.clone()]).await;

    assert!(hub.kept(&mouse).is_none(), "the background pass read the mouse (#567)");
    assert!(hub.kept(&controller).is_some(), "the controller is still read in the background");
    assert_eq!(read, 1, "one device read: the controller");
    assert_eq!(hub.background_readable(vec![mouse.clone(), controller.clone()]).await, vec![controller]);
}

#[tokio::test]
async fn read_again_still_reads_the_mouse() {
    let hub = PeripheralHub::with_idle(fake_peripherals(), away());
    let mouse = id_of(&hub, PeripheralKind::Mouse).await;
    hub.apply(&mouse, PeripheralAccess { reads: true, writes: false }).await.expect("the fake opens");

    hub.read_fresh(&mouse).await.expect("the person's Read again reads the mouse");

    assert!(hub.kept(&mouse).is_some(), "Read again keeps what it read");
}

#[test]
fn the_real_x6_opts_out_and_the_real_8bitdo_does_not() {
    let x6 = canvas_adapters::attack_shark_x6::AttackSharkX6::new();
    let pad = canvas_adapters::eightbitdo_ultimate2::EightBitDoUltimate2::new();
    assert!(!Peripheral::background_reads(&x6), "#567: the X6 is read only by Read again");
    assert!(Peripheral::background_reads(&pad));
}
