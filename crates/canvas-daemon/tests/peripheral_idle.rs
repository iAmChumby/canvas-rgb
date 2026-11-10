//! #127: a device read happens in the background, and only after Windows reports no keyboard or
//! mouse input for ten seconds. The idle clock is a fake here, so the tests step it.
//!
//! No hardware is touched: the peripheral is the fake in `common`, which records every read.

mod common;

use canvas_daemon::peripherals::{InputIdle, Peripheral, PeripheralHub};
use canvas_proto::peripheral::PeripheralAccess;
use common::FakePeripheral;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A fake input clock: the milliseconds of idle time the test last stored.
fn fake_idle() -> (Arc<AtomicU64>, Arc<dyn InputIdle>) {
    let millis = Arc::new(AtomicU64::new(0));
    let clock = millis.clone();
    let idle: Arc<dyn InputIdle> =
        Arc::new(move || Duration::from_millis(clock.load(Ordering::SeqCst)));
    (millis, idle)
}

/// A hub with the fake peripheral and the test's input clock.
fn hub_with(fake: &FakePeripheral, idle: Arc<dyn InputIdle>) -> PeripheralHub {
    PeripheralHub::with_idle(
        vec![Box::new(fake.clone()) as Box<dyn Peripheral>],
        idle,
    )
}

/// The device is open for reads, but nothing has been read yet.
async fn reads_on(hub: &PeripheralHub, fake: &FakePeripheral) {
    hub.apply(&fake.id(), PeripheralAccess { reads: true, writes: false })
        .await
        .expect("the fake opens");
}

#[tokio::test]
async fn a_background_read_waits_for_ten_seconds_of_no_input_and_then_reads_once() {
    let fake = FakePeripheral::default();
    let (millis, idle) = fake_idle();
    let hub = hub_with(&fake, idle);
    let id = fake.id();
    reads_on(&hub, &fake).await;

    // The person used the mouse three seconds ago: nothing is read.
    millis.store(3_000, Ordering::SeqCst);
    assert_eq!(hub.refresh_idle(std::slice::from_ref(&id)).await, 0);
    assert_eq!(fake.log().reads, 0, "three seconds of idleness is not enough");

    // Ten seconds with no keyboard or mouse input: exactly one read.
    millis.store(10_000, Ordering::SeqCst);
    assert_eq!(hub.refresh_idle(std::slice::from_ref(&id)).await, 1);
    assert_eq!(fake.log().reads, 1, "ten seconds idle: exactly one device read");

    // The read is kept and fresh now: a later pass reads nothing.
    assert_eq!(hub.refresh_idle(std::slice::from_ref(&id)).await, 0);
    assert_eq!(fake.log().reads, 1, "the values are kept: no second read");
}

/// F3: an adapter whose enumeration failed said nothing about its devices, so their kept values
/// stay (the hub-level checks live here because they read the kept map directly).
#[tokio::test]
async fn a_failed_enumeration_keeps_the_values_of_that_adapters_devices() {
    let fake = FakePeripheral::default();
    let hub = PeripheralHub::new(vec![Box::new(fake.clone()) as Box<dyn Peripheral>]);
    let id = fake.id();
    hub.apply(&id, PeripheralAccess { reads: true, writes: false }).await.unwrap();
    hub.read_fresh(&id).await.unwrap();
    fake.0.lock().unwrap().fail_discover = true;
    assert!(hub.discover_now().await.is_empty());
    assert!(hub.kept(&id).is_some(), "the scan failed: the values stay");
    assert_eq!(fake.log().reads, 1, "the failed scan is no read");
}

/// A device that drops out of the enumeration loses its kept values: when it comes back, the
/// next read is a device read again.
#[tokio::test]
async fn a_device_that_leaves_the_enumeration_loses_its_kept_values() {
    let fake = FakePeripheral::default();
    let hub = PeripheralHub::new(vec![Box::new(fake.clone()) as Box<dyn Peripheral>]);
    let id = fake.id();
    hub.apply(&id, PeripheralAccess { reads: true, writes: false }).await.unwrap();
    hub.discover().await;
    hub.read_fresh(&id).await.unwrap();
    assert_eq!(fake.log().reads, 1);
    assert!(hub.kept(&id).is_some(), "the read is kept");
    // The device is unplugged and the enumeration notices; then it is plugged back in.
    let found = std::mem::take(&mut fake.0.lock().unwrap().found);
    hub.discover_now().await;
    assert!(hub.kept(&id).is_none(), "the kept values were dropped with the device");
    fake.0.lock().unwrap().found = found;
    hub.discover_now().await;
    hub.read_fresh(&id).await.unwrap();
    assert_eq!(fake.log().reads, 2, "the values were read again after the device came back");
}

#[tokio::test]
async fn input_that_arrives_before_the_read_abandons_it() {
    let fake = FakePeripheral::default();
    // The first question is answered "ten seconds idle"; the person moves before the read, so
    // the second question is answered zero.
    let asked = Arc::new(AtomicU64::new(0));
    let clock = asked.clone();
    let idle: Arc<dyn InputIdle> = Arc::new(move || {
        if clock.fetch_add(1, Ordering::SeqCst) == 0 {
            Duration::from_secs(10)
        } else {
            Duration::ZERO
        }
    });
    let hub = hub_with(&fake, idle);
    let id = fake.id();
    reads_on(&hub, &fake).await;

    assert_eq!(hub.refresh_idle(std::slice::from_ref(&id)).await, 0);
    assert_eq!(
        fake.log().reads,
        0,
        "a read is abandoned before it starts when input arrived meanwhile"
    );
}
