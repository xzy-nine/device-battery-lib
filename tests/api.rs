//! 导出层的契约测试。
//!
//! 覆盖类型映射、下标查找与错误分支；不依赖真实 HID 设备，
//! 枚举本身的实际行为由 `examples/demo.rs` 在真机上冒烟。

use device_battery_lib::api::{
    BatteryDeviceInfo, BatteryError, BatteryLevelKind, BatterySnapshot, ChargeStatus, DeviceVendor,
};

/// 构造一条设备快照，字段取值互不相同以便断言逐字段映射。
fn device(
    vendor_id: u16,
    product_id: u16,
    slot: u8,
    online: bool,
    has_battery: bool,
    percentage: u8,
) -> BatteryDeviceInfo {
    BatteryDeviceInfo {
        vendor: DeviceVendor::Logitech,
        vendor_id,
        product_id,
        name: format!("device-{slot}"),
        slot,
        online,
        has_battery,
        percentage,
        level: BatteryLevelKind::Good,
        status: ChargeStatus::Charging,
    }
}

fn snapshot(devices: Vec<BatteryDeviceInfo>) -> BatterySnapshot {
    BatterySnapshot { devices }
}

#[test]
fn read_battery_returns_the_requested_device() {
    let first = device(0x046d, 0xc52b, 1, true, true, 100);
    let second = device(0x046d, 0xc08d, 2, true, true, 55);
    let list = snapshot(vec![first, second]);

    let hit = list.read_battery(1).expect("第二个设备可读");
    assert_eq!(hit.slot, 2);
    assert_eq!(hit.percentage, 55);
}

#[test]
fn read_battery_rejects_index_past_the_end() {
    let list = snapshot(vec![device(0x046d, 0xc52b, 1, true, true, 100)]);

    match list.read_battery(999) {
        Err(BatteryError::IndexOutOfRange { index }) => assert_eq!(index, 999),
        other => panic!("越界应返回 IndexOutOfRange，实际 {other:?}"),
    }
}

#[test]
fn read_battery_rejects_negative_index() {
    let list = snapshot(vec![device(0x046d, 0xc52b, 1, true, true, 100)]);

    match list.read_battery(-1) {
        Err(BatteryError::IndexOutOfRange { index }) => assert_eq!(index, -1),
        other => panic!("负数下标应返回 IndexOutOfRange，实际 {other:?}"),
    }
}

#[test]
fn empty_snapshot_has_no_devices() {
    let list = snapshot(Vec::new());

    assert!(list.devices.is_empty());
    assert!(matches!(
        list.read_battery(0),
        Err(BatteryError::IndexOutOfRange { .. })
    ));
}

#[test]
fn device_keeps_u16_and_u8_ranges() {
    let max = device(0xffff, 0xffff, 255, false, false, 200);

    assert_eq!(max.vendor_id, 0xffff);
    assert_eq!(max.product_id, 0xffff);
    assert_eq!(max.slot, 255);
    assert_eq!(max.percentage, 200);
}

#[test]
fn offline_device_without_battery_is_preserved() {
    let offline = device(0x046d, 0xc52b, 0, false, false, 0);

    assert!(!offline.online);
    assert!(!offline.has_battery);
}

#[test]
fn error_messages_are_stable() {
    let list = snapshot(Vec::new());
    let error = list.read_battery(7).unwrap_err();

    assert_eq!(error.to_string(), "index out of range: 7");
}
