# device-battery-lib 接口文档

`device-battery-lib` 经 UniFFI 导出跨语言接口，可供 Kotlin、Swift、Python 等语言调用。库内置两条互不相干的设备后端：罗技走 HID++，飞智走私有报文协议；两者共用同一套导出的接口。

## 后端

| 厂商 | 协议 | 电量来源 |
|------|------|----------|
| 罗技 | HID++ 0x1000 / 0x1001 等特性 | 设备直接上报百分比（0-100） |
| 飞智 | Space Station 服务命名管道 | 服务上报的档位（一档 20%） |

枚举时两条后端各自独立执行，任一后端失败都只贡献空结果，不影响另一后端。

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
| `deviceId` | `String` | 稳定标识，见下节 |
| `vendorId` | `UShort` | USB Vendor ID（`0x046D` 罗技，`0x045E` 飞智经接收器） |
| `productId` | `UShort` | USB Product ID |
| `name` | `String` | 设备名称 |
| `slot` | `UByte` | 接收器配对槽位（1..=6；直连设备与飞智为 0） |
| `online` | `Boolean` | 是否在线 |
| `hasBattery` | `Boolean` | 是否有电量数据 |
| `percentage` | `UByte` | 电量百分比 0-100 |
| `level` | `BatteryLevelKind` | 电量等级 |
| `status` | `ChargeStatus` | 充电状态 |

飞智只上报档位而非百分比，`percentage` 按一档 20% 换算：档位 0 → 0%、1 → 20%、4 → 80%，5 档及以上按 100% 截断。服务不区分充电与满电，档位满时 `status` 记 `FULL`，其余记 `DISCHARGING`。

### 设备标识

`deviceId` 在一次枚举内唯一、跨枚举稳定，用于刷新前后比对与自定义名的键。格式带厂商前缀：

| 厂商 | 格式 | 示例 |
|------|------|------|
| 罗技 | `logitech:VID:PID:槽位` | `logitech:046d:c52b:1` |
| 飞智 | `flydigi:UID` | `flydigi:3891212c` |

飞智不能用 `VID:PID:槽位`：接收器模式下它复用微软的通用 VID/PID（`0x045E:0x028E`），槽位恒为 0，会与真正的 Xbox 手柄撞车。服务返回的 `uid` 才是该设备的唯一标识。

### `BatteryLevelKind`

电量等级：`CRITICAL`、`LOW`、`GOOD`、`FULL`、`UNKNOWN`。飞智按换算后的百分比归类：0-10 → `CRITICAL`，11-30 → `LOW`，31-80 → `GOOD`，其余 → `FULL`。

### `ChargeStatus`

充电状态：`DISCHARGING`、`CHARGING`、`CHARGING_SLOW`、`FULL`、`ERROR`、`UNKNOWN`。

### `BatterySnapshot`

一次枚举的结果，字段 `devices` 为 `List<BatteryDeviceInfo>`。

### `BatteryException`

枚举错误，三个变体：

| 变体 | 含义 |
|------|------|
| `Enumerate(detail)` | 罗技后端枚举失败，`detail` 为原生错误描述 |
| `IndexOutOfRange(index)` | 请求的下标越界或为负 |
| `NoBatteryData` | 设备无电量数据 |

飞智后端失败不抛异常，只产出空结果。

## 接口

### `BatteryLib()`

创建入口对象。构造本身不触达硬件。

### `suspend enumerateDevices(): BatterySnapshot`

枚举罗技与飞智设备及其电量快照。接口为异步：底层探测耗时数百毫秒到数秒，由调用方的调度器承载，不阻塞调用线程。

### `BatterySnapshot.readBattery(index: Int): BatteryDeviceInfo`

取指定下标设备的电量快照。`index` 为 `enumerateDevices` 返回列表中的下标；越界或为负时抛 `BatteryException.IndexOutOfRange`。

## 使用示例

### Kotlin

```kotlin
import kotlinx.coroutines.runBlocking
import notifyrelay.battery.ffi.BatteryLib

val snapshot = runBlocking {
    BatteryLib().enumerateDevices()
}
for (device in snapshot.devices) {
    println("${device.deviceId} ${device.name}: ${device.percentage.toInt()}% ${device.level} ${device.status}")
}
```

### Rust

```rust
use device_battery_lib::api::BatteryLib;

let lib = BatteryLib::new();
let snapshot = lib.enumerate_devices().await?;
for device in &snapshot.devices {
    println!("{} {}: {}% {:?} {:?}", device.device_id, device.name, device.percentage, device.level, device.status);
}
```

## 飞智服务管道

飞智把设备电量算在它自己的 Space Station 服务里，经命名管道对外提供；官方界面与第三方工具都只是这条管道的客户端。实现在 [`src/flydigi.rs`](src/flydigi.rs)，要点：

- 管道为 `\\.\pipe\fcs.sock`，服务未安装或未运行时不存在，此时返回空列表。
- 分帧：`FDG_PROTOCOL\n` + `i32` 小端长度 + protobuf 消息。
- 请求 `IpcRequest{ cmd = 4097 }`（GetConnectedDevices）取设备列表。
- 响应 `ConnectedDevicesResponse` 内每条 `DeviceInfo` 的字段：1 productName、3 connectType、4 deviceCode、7 uid、8 isConnected、9 battery、11 vendorId、12 productId。
- `battery` 是档位而非百分比，一档 20%。
- 管道实例有限：句柄读完即释放，遇到 `ERROR_PIPE_BUSY` 时短暂等待后重试一次。

本库不触碰 HID 与飞智的内核驱动，也不依赖其闭源二进制。

## 线程安全

- 同一进程内可安全地从多个线程并发调用枚举函数（内部每次调用创建独立的 tokio 运行时）。
- 枚举结果按值返回，不存在需要调用方释放的裸指针或句柄。
