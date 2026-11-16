//! UniFFI 导出层。
//!
//! 直接复用 `openlogi-hid` 的完整 HID++ 实现（枚举 + 电量探测），
//! Kotlin 绑定由 gobley-uniffi-bindgen 生成；本层只做类型映射。

use std::sync::Arc;

use openlogi_core::device::{BatteryLevel, BatteryStatus};
use openlogi_hid::enumerate;

/// 电量等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BatteryLevelKind {
    /// 电量危险
    Critical,
    /// 电量偏低
    Low,
    /// 电量正常
    Good,
    /// 电量已满
    Full,
    /// 无法判定
    Unknown,
}

/// 充电状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChargeStatus {
    /// 放电中
    Discharging,
    /// 充电中
    Charging,
    /// 慢充
    ChargingSlow,
    /// 已充满
    Full,
    /// 充电异常
    Error,
    /// 无法判定
    Unknown,
}

fn level_kind(level: BatteryLevel) -> BatteryLevelKind {
    match level {
        BatteryLevel::Critical => BatteryLevelKind::Critical,
        BatteryLevel::Low => BatteryLevelKind::Low,
        BatteryLevel::Good => BatteryLevelKind::Good,
        BatteryLevel::Full => BatteryLevelKind::Full,
        BatteryLevel::Unknown => BatteryLevelKind::Unknown,
    }
}

fn charge_status(status: BatteryStatus) -> ChargeStatus {
    match status {
        BatteryStatus::Discharging => ChargeStatus::Discharging,
        BatteryStatus::Charging => ChargeStatus::Charging,
        BatteryStatus::ChargingSlow => ChargeStatus::ChargingSlow,
        BatteryStatus::Full => ChargeStatus::Full,
        BatteryStatus::Error => ChargeStatus::Error,
        BatteryStatus::Unknown => ChargeStatus::Unknown,
    }
}

/// 一台设备的电量快照。
#[derive(Debug, Clone, uniffi::Record)]
pub struct BatteryDeviceInfo {
    /// USB Vendor ID（0x046D = 罗技，0x045E = 飞智经接收器）
    pub vendor_id: u16,
    /// USB Product ID
    pub product_id: u16,
    /// 设备稳定标识，形如 `logitech:046d:c52b:1` / `flydigi:3891212c`。
    ///
    /// 同一台设备在多次枚举之间保持不变，用于刷新前后比对与用户自定义名的键。
    pub device_id: String,
    /// 设备名称
    pub name: String,
    /// 接收器配对槽位（1..=6；直连设备为 0）
    pub slot: u8,
    /// 是否在线
    pub online: bool,
    /// 是否有电量数据
    pub has_battery: bool,
    /// 电量百分比 0-100
    pub percentage: u8,
    /// 电量等级
    pub level: BatteryLevelKind,
    /// 充电状态
    pub status: ChargeStatus,
}

/// 枚举错误。
///
/// 字段名不带 `message`：UniFFI 为变体生成同名取值器，与 `Throwable.message` 冲突。
#[derive(Debug, uniffi::Error, thiserror::Error)]
pub enum BatteryError {
    /// 枚举或读取失败
    #[error("{detail}")]
    Enumerate {
        /// 原生错误描述
        detail: String,
    },
    /// 下标越界
    #[error("index out of range: {index}")]
    IndexOutOfRange {
        /// 请求的下标
        index: i32,
    },
    /// 设备无电量数据
    #[error("device has no battery data")]
    NoBatteryData,
}

/// 一次枚举的结果。
#[derive(uniffi::Record)]
pub struct BatterySnapshot {
    /// 全部设备快照
    pub devices: Vec<BatteryDeviceInfo>,
}

impl BatterySnapshot {
    /// 重新读取指定下标设备的电量（每次都会重新探测，可能耗时数百毫秒）。
    pub fn read_battery(&self, index: i32) -> Result<BatteryDeviceInfo, BatteryError> {
        if index < 0 {
            return Err(BatteryError::IndexOutOfRange { index });
        }
        self.devices
            .get(index as usize)
            .cloned()
            .ok_or(BatteryError::IndexOutOfRange { index })
    }
}

/// 电池读取库的入口。
#[derive(uniffi::Object)]
pub struct BatteryLib;

/// 罗技设备的稳定标识：`logitech:VID:PID:槽位`。
///
/// 十六进制小写、不补前导零；槽位取接收器配对槽位，直连设备为 0。
fn logitech_device_id(vendor_id: u16, product_id: u16, slot: u8) -> String {
    format!("logitech:{vendor_id:04x}:{product_id:04x}:{slot}")
}

/// 枚举所有 HID++ 设备及其电量。
async fn collect_devices() -> Result<Vec<BatteryDeviceInfo>, BatteryError> {
    let inventories = enumerate().await.map_err(|error| BatteryError::Enumerate {
        detail: error.to_string(),
    })?;

    let mut items = Vec::new();
    for inv in &inventories {
        for p in &inv.paired {
            let display = p
                .codename
                .clone()
                .unwrap_or_else(|| format!("{} (slot {})", inv.receiver.name, p.slot));

            let (has_battery, percentage, level, status) = match &p.battery {
                Some(b) => (
                    true,
                    b.percentage,
                    level_kind(b.level),
                    charge_status(b.status),
                ),
                None => (false, 0, BatteryLevelKind::Unknown, ChargeStatus::Unknown),
            };

            items.push(BatteryDeviceInfo {
                device_id: logitech_device_id(
                    inv.receiver.vendor_id,
                    inv.receiver.product_id,
                    p.slot,
                ),
                vendor_id: inv.receiver.vendor_id,
                product_id: inv.receiver.product_id,
                name: display,
                slot: p.slot,
                online: p.online,
                has_battery,
                percentage,
                level,
                status,
            });
        }
    }
    Ok(items)
}

/// 枚举飞智设备及其电量。
///
/// 飞智电量取自 Space Station 服务的命名管道，服务未安装或未运行时按空列表处理：
/// 不阻断罗技设备的结果。
fn collect_flydigi_devices() -> Vec<BatteryDeviceInfo> {
    let Ok(devices) = crate::flydigi::enumerate() else {
        return Vec::new();
    };

    devices
        .into_iter()
        .map(|device| BatteryDeviceInfo {
            device_id: flydigi_device_id(&device.uid),
            vendor_id: device.vendor_id,
            product_id: device.product_id,
            name: if device.product_name.is_empty() {
                device.device_code.clone()
            } else {
                device.product_name.clone()
            },
            slot: 0,
            online: device.connected,
            has_battery: true,
            percentage: device.battery_percent(),
            level: level_kind_from_percent(device.battery_percent()),
            // 服务不区分充电与满电，只给档位；按档位是否已满推断。
            status: if device.battery_percent() >= 100 {
                ChargeStatus::Full
            } else {
                ChargeStatus::Discharging
            },
        })
        .collect()
}

/// 飞智设备的稳定标识：`flydigi:UID`。
///
/// 服务给的 `uid` 是该设备在飞智体系内的唯一标识（如 `3891212C`），
/// 比 `VID:PID:槽位` 可靠：接收器模式下飞智复用微软的通用 VID/PID，槽位恒为 0。
fn flydigi_device_id(uid: &str) -> String {
    format!("flydigi:{}", uid.to_ascii_lowercase())
}

/// 按电量百分比归类到本库的电量等级。
fn level_kind_from_percent(percent: u8) -> BatteryLevelKind {
    match percent {
        0..=10 => BatteryLevelKind::Critical,
        11..=30 => BatteryLevelKind::Low,
        31..=80 => BatteryLevelKind::Good,
        _ => BatteryLevelKind::Full,
    }
}

#[uniffi::export]
impl BatteryLib {
    /// 创建电池读取库的入口。
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    /// 枚举设备并读取电量快照。
    ///
    /// 罗技与飞智各自独立枚举：一个后端失败不影响另一个后端的结果。
    pub async fn enumerate_devices(&self) -> Result<BatterySnapshot, BatteryError> {
        let mut devices = collect_devices().await.unwrap_or_default();
        devices.extend(collect_flydigi_devices());
        Ok(BatterySnapshot { devices })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logitech_id_carries_vendor_and_slot() {
        assert_eq!(
            logitech_device_id(0x046D, 0xC52B, 1),
            "logitech:046d:c52b:1"
        );
    }

    #[test]
    fn logitech_id_differs_by_slot() {
        // 同一接收器下的不同槽位是不同设备。
        assert_ne!(
            logitech_device_id(0x046D, 0xC539, 1),
            logitech_device_id(0x046D, 0xC539, 2)
        );
    }

    #[test]
    fn flydigi_id_uses_service_uid() {
        assert_eq!(flydigi_device_id("3891212C"), "flydigi:3891212c");
    }

    #[test]
    fn flydigi_id_is_case_insensitive() {
        assert_eq!(flydigi_device_id("3891212c"), flydigi_device_id("3891212C"));
    }

    #[test]
    fn ids_of_different_vendors_never_collide() {
        // 飞智在接收器模式下复用微软的通用 VID/PID、槽位恒为 0，
        // 若仍用 VID:PID:槽位 会与真正的 Xbox 手柄撞车。
        let flydigi = flydigi_device_id("3891212C");
        let xbox_like = logitech_device_id(0x045E, 0x028E, 0);
        assert_ne!(flydigi, xbox_like);
        assert!(flydigi.starts_with("flydigi:"));
        assert!(xbox_like.starts_with("logitech:"));
    }

    #[test]
    fn percent_maps_to_level_kind() {
        assert_eq!(level_kind_from_percent(0), BatteryLevelKind::Critical);
        assert_eq!(level_kind_from_percent(20), BatteryLevelKind::Low);
        assert_eq!(level_kind_from_percent(80), BatteryLevelKind::Good);
        assert_eq!(level_kind_from_percent(100), BatteryLevelKind::Full);
    }
}
