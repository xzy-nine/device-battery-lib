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
    /// USB Vendor ID（0x046D = Logitech）
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

#[uniffi::export]
impl BatteryLib {
    /// 创建电池读取库的入口。
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    /// 枚举设备并读取电量快照。
    pub fn enumerate_devices(&self) -> Result<BatterySnapshot, BatteryError> {
        Ok(BatterySnapshot {
            devices: collect_devices()?,
        })
    }
}
