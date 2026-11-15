//! 电池设备电量读取库
//!
//! 从 OpenLogi 提取，直接复用其 HID++ 协议层（`openlogi-hidpp`）、
//! 设备层（`openlogi-device`）与主机后端（`openlogi-hid`）。
//! 跨语言接口经 UniFFI 导出，见 [`api`]。

/// 重新导出 openlogi-hid 的全部公开 API
/// （含其转发的 openlogi-device / hidpp 类型）
pub use openlogi_hid::*;

/// openlogi-core 的类型（DeviceInventory、PairedDevice、BatteryInfo 等）
pub use openlogi_core::*;

/// UniFFI 导出层（BatteryLib 与相关类型）
pub mod api;

// 静态链接时必须有一个被引用的符号，否则 UniFFI 的导出模块会参与裁剪。
uniffi::setup_scaffolding!();
