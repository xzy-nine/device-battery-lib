# device-battery-lib

游戏外设电量读取库（UniFFI 导出，提供 `cdylib` / `staticlib`）。

内置两条互不相干的设备后端，各自实现一门厂商私有协议：

| 厂商 | 协议 | 实现 |
|------|------|------|
| 罗技 | HID++（`openlogi-hidpp`） | `api`，基于 OpenLogi 裁剪的 crate |
| 飞智 | Space Station 服务管道 | [`flydigi`](src/flydigi.rs) |

两者经同一套 UniFFI 接口对外，`BatteryDeviceInfo.vendor` 区分来源；枚举时各自独立，一个后端失败不影响另一个后端的结果。

飞智的电量由它自己的 Space Station 服务算出，经命名管道 `\\.\pipe\fcs.sock` 对外提供；官方界面与第三方工具都只是这条管道的客户端。本库实现该客户端，不碰 HID、不装驱动。服务未安装或未运行时管道不存在，飞智设备按空列表处理。

## 开源协议（License Notice）

- 上游项目 OpenLogi 采用 **MIT OR Apache-2.0** 双许可。本仓库遵守其协议要求，保留同等双许可：
  - [LICENSE-MIT](LICENSE-MIT)
  - [LICENSE-APACHE](LICENSE-APACHE)
  - 本仓库整体以 `MIT OR Apache-2.0` 分发，原始版权声明保留。
- `crates/openlogi-hidpp` 是 [`hidpp`](https://crates.io/crates/hidpp) 协议 crate 的硬分支，原 crate 采用 **0BSD** 许可，其声明见 [crates/openlogi-hidpp/LICENSE](crates/openlogi-hidpp/LICENSE)。
- 感谢上游 OpenLogi 及 `hidpp` 项目作者的贡献。

## 构建

```sh
cargo build --release
```

产物位于 `target/release/`（`device_battery_lib.dll` / `.so` / `.a`）。

接口经 UniFFI 导出，Rust 侧入口为 `api::BatteryLib`：

```rust
use device_battery_lib::api::BatteryLib;

let lib = BatteryLib::new();
let snapshot = lib.enumerate_devices()?;
for device in &snapshot.devices {
    println!("{:?} {}: {}%", device.vendor, device.name, device.percentage);
}
```

Kotlin 绑定由 `gobley-uniffi-bindgen` 依 `uniffi.toml` 生成，接口与数据类型的完整说明见 [docs/api.md](docs/api.md)。

## 各后端单独使用

不经过 UniFFI 时，两条后端各自有独立入口：

```rust
// 罗技：枚举接收器及其配对设备
let inventories = device_battery_lib::enumerate().await?;

// 飞智：经 Space Station 服务管道读取电量
let devices = device_battery_lib::flydigi::enumerate()?;
```

飞智只提供档位而非百分比，`FlydigiDevice::battery_percent` 按一档 20% 换算；罗技的百分比由设备直接上报。
