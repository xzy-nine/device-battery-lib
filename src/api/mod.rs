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

/// 设备所属厂商。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceVendor {
    /// 罗技（HID++）
    Logitech,
    /// 飞智
    Flydigi,
}

/// 一台设备的电量快照。
#[derive(Debug, Clone, uniffi::Record)]
pub struct BatteryDeviceInfo {
    /// 设备所属厂商
    pub vendor: DeviceVendor,
    /// USB Vendor ID（0x046D = Logitech，0x37D7 = Flydigi）
    pub vendor_id: u16,
    /// USB Product ID
    pub product_id: u16,
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

fn with_runtime<T, F>(f: F) -> Result<T, BatteryError>
where
    F: std::future::Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| BatteryError::Enumerate {
            detail: e.to_string(),
        })?;
    rt.block_on(f).map_err(|e| BatteryError::Enumerate {
        detail: e.to_string(),
    })
}

/// 枚举所有 HID++ 设备及其电量。
fn collect_devices() -> Result<Vec<BatteryDeviceInfo>, BatteryError> {
    let inventories = with_runtime(async { Ok(enumerate().await?) })?;

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
                vendor: DeviceVendor::Logitech,
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
            vendor: DeviceVendor::Flydigi,
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
    pub fn enumerate_devices(&self) -> Result<BatterySnapshot, BatteryError> {
        let mut devices = collect_devices().unwrap_or_default();
        devices.extend(collect_flydigi_devices());
        Ok(BatterySnapshot { devices })
    }
}
