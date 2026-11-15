# device-battery-lib 接口文档

`device-battery-lib` 经 UniFFI 导出跨语言接口，可供 Kotlin、Swift、Python 等语言调用，底层复用 OpenLogi 的完整 HID++ 实现，无需自行处理协议细节。

## 构建

```bash
cargo build --release
```

产物：

| 平台 | 文件 |
|------|------|
| Windows | `target/release/device_battery_lib.dll`（另有 `.lib` 导入库） |
| Linux | `target/release/libdevice_battery_lib.so` |
| macOS | `target/release/libdevice_battery_lib.dylib` |

> 注意：库内部会创建 tokio 运行时并阻塞等待枚举/读取完成，单个调用耗时约数百毫秒到数秒，请勿在 UI 主线程中高频调用。

## 绑定生成

Kotlin 绑定由 `gobley-uniffi-bindgen` 依据 `uniffi.toml` 生成：

```bash
gobley-uniffi-bindgen --library target/release/device_battery_lib.dll \
  --config uniffi.toml --out-dir build/generated/uniffi
```

`uniffi.toml` 的 `package_name` 为 `notifyrelay.battery.ffi`，`cdylib_name` 为 `device_battery_lib`。该库只被桌面端 JVM 消费，故 `kotlin_targets` 只列 `jvm`。

## 数据类型

### `BatteryDeviceInfo`

一台设备的电量快照。

| 字段 | 类型 | 说明 |
|------|------|------|
| `vendorId` | `UShort` | USB Vendor ID（`0x046D` = Logitech） |
| `productId` | `UShort` | USB Product ID |
| `name` | `String` | 设备名称（绑定记录中为 `name`） |
| `slot` | `UByte` | 接收器配对槽位（1..=6；直连设备为 0） |
| `online` | `Boolean` | 是否在线 |
| `hasBattery` | `Boolean` | 是否有电量数据 |
| `percentage` | `UByte` | 电量百分比 0-100 |
| `level` | `BatteryLevelKind` | 电量等级 |
| `status` | `ChargeStatus` | 充电状态 |

### `BatteryLevelKind`

电量等级：`CRITICAL`、`LOW`、`GOOD`、`FULL`、`UNKNOWN`。

### `ChargeStatus`

充电状态：`DISCHARGING`、`CHARGING`、`CHARGING_SLOW`、`FULL`、`ERROR`、`UNKNOWN`。

### `BatterySnapshot`

一次枚举的结果，字段 `devices` 为 `List<BatteryDeviceInfo>`。

### `BatteryException`

枚举错误，三个变体：

| 变体 | 含义 |
|------|------|
| `Enumerate(detail)` | 枚举或读取失败，`detail` 为原生错误描述 |
| `IndexOutOfRange(index)` | 请求的下标越界或为负 |
| `NoBatteryData` | 设备无电量数据 |

## 接口

### `BatteryLib()`

创建入口对象。

### `enumerateDevices(): BatterySnapshot`

枚举所有 HID++ 设备及其电量快照。枚举失败时抛 `BatteryException.Enumerate`。

### `BatterySnapshot.readBattery(index: Int): BatteryDeviceInfo`

重新读取指定设备的电量（每次调用都会重新探测，可能耗时较长）。`index` 为 `enumerateDevices` 返回列表中的下标；越界或为负时抛 `BatteryException.IndexOutOfRange`。

## 使用示例

### Kotlin

```kotlin
import notifyrelay.battery.ffi.BatteryLib

val lib = BatteryLib()
val snapshot = lib.enumerateDevices()
for (device in snapshot.devices) {
    println("${device.name}: ${device.percentage.toInt()}% ${device.level} ${device.status}")
}
```

### Rust

```rust
use device_battery_lib::api::BatteryLib;

let lib = BatteryLib::new();
let snapshot = lib.enumerate_devices()?;
for device in &snapshot.devices {
    println!("{}: {}% {:?} {:?}", device.name, device.percentage, device.level, device.status);
}
```

## 线程安全

- 同一进程内可安全地从多个线程并发调用枚举/读取函数（内部每次调用创建独立的 tokio 运行时）。
- 枚举结果按值返回，不存在需要调用方释放的裸指针或句柄。
