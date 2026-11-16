//! 飞智（Flydigi）设备的电量读取。
//!
//! 飞智把设备电量算在它自己的 Space Station 服务里，并以 protobuf over 命名管道的形式
//! 对外提供；官方界面与第三方工具都只是这条管道的客户端。本模块实现该客户端，
//! 不触碰 HID 与驱动，也不依赖飞智的闭源二进制。
//!
//! 分帧为 `FDG_PROTOCOL\\n` + `i32` 小端长度 + protobuf 消息。请求是一条
//! `IpcRequest{cmd}`，响应的设备列表在 `ConnectedDevicesResponse` 里。
//!
//! 服务未安装或未运行时管道不存在，此时[enumerate]返回空列表：调用方按"本机没有飞智
//! 设备"处理即可，不影响其它厂商的后端。

use std::io::{Read, Write};
use std::time::Duration;

use thiserror::Error;

/// 服务暴露设备列表的命名管道。
const PIPE_PATH: &str = r"\\.\pipe\fcs.sock";

/// 分帧前导，与 `FDG_PROTOCOL` 一致。
const PROTOCOL_HEADER: &[u8] = b"FDG_PROTOCOL\n";

/// `IpcCommandEnum_GetConnectedDevices`：请求已连接设备及其电量。
const CMD_GET_CONNECTED_DEVICES: u64 = 4097;

/// 管道繁忙（`ERROR_PIPE_BUSY`）时最多等待的时长。
const PIPE_BUSY_WAIT: Duration = Duration::from_millis(1_000);

/// 读响应的上限，避免服务端异常时无限阻塞。
const MAX_RESPONSE_BYTES: usize = 1 << 20;

/// 一个档位对应的百分比。
///
/// 服务上报的是档位而非百分比：实测档位 1 对应 20%、4 对应 80%，故一档 20%。
const PERCENT_PER_LEVEL: u32 = 20;

/// 读取飞智设备电量的失败原因。
#[derive(Debug, Error)]
pub enum FlydigiError {
    /// 与服务的管道通信失败。
    #[error("pipe error: {0}")]
    Io(#[from] std::io::Error),
    /// 响应不是合法的分帧结构。
    #[error("malformed frame: {0}")]
    Frame(&'static str),
    /// 响应里的 protobuf 结构无法解析。
    #[error("malformed protobuf: {0}")]
    Protobuf(&'static str),
}

/// 一个飞智设备的电量快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlydigiDevice {
    /// 产品名，例如 `Controller (Flydigi VADER3)`。
    pub product_name: String,
    /// 设备代号，例如 `f3p`。
    pub device_code: String,
    /// 设备稳定标识，例如 `3891212C`。
    pub uid: String,
    /// USB Vendor ID。
    pub vendor_id: u16,
    /// USB Product ID。
    pub product_id: u16,
    /// 连接方式。
    pub connect_type: ConnectType,
    /// 是否在线。
    pub connected: bool,
    /// 电量档位；服务未上报时为 0。
    pub battery_level: u8,
}

impl FlydigiDevice {
    /// 电量百分比。
    ///
    /// 档位粒度是 20%：0 → 0%、1 → 20%、4 → 80%；5 档及以上按 100% 截断。
    #[must_use]
    pub fn battery_percent(&self) -> u8 {
        u32::from(self.battery_level)
            .saturating_mul(PERCENT_PER_LEVEL)
            .min(100) as u8
    }
}

/// 设备与主机的连接方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectType {
    /// 有线直连。
    Wired,
    /// 通过 2.4G 接收器。
    Receiver,
    /// 其它或服务未上报。
    Unknown,
}

/// 枚举本机的飞智设备并读取电量。
///
/// 服务未安装、未运行或管道被独占时返回空列表。单个设备的解析失败会被跳过。
pub fn enumerate() -> Result<Vec<FlydigiDevice>, FlydigiError> {
    let response = request_connected_devices()?;
    Ok(parse_connected_devices(&response))
}

/// 向服务请求设备列表并返回响应体（已去掉分帧头）。
fn request_connected_devices() -> Result<Vec<u8>, FlydigiError> {
    let mut pipe = open_pipe()?;

    // IpcRequest{ cmd: 4097 }
    let mut payload = Vec::new();
    payload.push(0x08);
    write_varint(&mut payload, CMD_GET_CONNECTED_DEVICES);

    let mut frame = Vec::with_capacity(PROTOCOL_HEADER.len() + 4 + payload.len());
    frame.extend_from_slice(PROTOCOL_HEADER);
    frame.extend_from_slice(&(payload.len() as i32).to_le_bytes());
    frame.extend_from_slice(&payload);

    pipe.write_all(&frame)?;
    pipe.flush()?;

    let mut buffer = vec![0u8; MAX_RESPONSE_BYTES];
    let read = pipe.read(&mut buffer)?;
    // 句柄随作用域释放：服务为每个客户端只开有限实例，占着会让后续调用撞 ERROR_PIPE_BUSY。
    drop(pipe);

    strip_frame(&buffer[..read])
}

/// 打开管道；繁忙时短暂等待后重试一次。
fn open_pipe() -> Result<std::fs::File, FlydigiError> {
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_PATH)
    {
        Ok(file) => Ok(file),
        Err(error) if error.raw_os_error() == Some(231) => {
            // ERROR_PIPE_BUSY：服务的实例都被占用，等一个空位再试。
            std::thread::sleep(PIPE_BUSY_WAIT);
            Ok(std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(PIPE_PATH)?)
        }
        Err(error) => Err(FlydigiError::Io(error)),
    }
}

/// 去掉协议头与长度前缀。
fn strip_frame(data: &[u8]) -> Result<Vec<u8>, FlydigiError> {
    let header_len = PROTOCOL_HEADER.len();
    if data.len() < header_len + 4 {
        return Err(FlydigiError::Frame("shorter than header"));
    }
    if !data.starts_with(PROTOCOL_HEADER) {
        return Err(FlydigiError::Frame("missing protocol header"));
    }
    let declared = i32::from_le_bytes([
        data[header_len],
        data[header_len + 1],
        data[header_len + 2],
        data[header_len + 3],
    ]);
    if declared < 0 {
        return Err(FlydigiError::Frame("negative length"));
    }
    let start = header_len + 4;
    let end = (start + declared as usize).min(data.len());
    Ok(data[start..end].to_vec())
}

/// 解析 `ConnectedDevicesResponse`。
///
/// 结构为 `IpcResponse{ f1: cmd, f2: ret, f5: ConnectedDevicesResponse{ repeated f1: DeviceInfo } }`。
fn parse_connected_devices(body: &[u8]) -> Vec<FlydigiDevice> {
    let mut devices = Vec::new();
    for field in parse_fields(body) {
        if field.number() != 5 {
            continue;
        }
        let Some(value) = field.bytes() else { continue };
        for inner in parse_fields(value) {
            if inner.number() != 1 {
                continue;
            }
            let Some(device) = inner.bytes() else {
                continue;
            };
            if let Some(parsed) = parse_device(device) {
                devices.push(parsed);
            }
        }
    }
    devices
}

/// 解析一条 `DeviceInfo`。
fn parse_device(data: &[u8]) -> Option<FlydigiDevice> {
    let mut device = FlydigiDevice {
        product_name: String::new(),
        device_code: String::new(),
        uid: String::new(),
        vendor_id: 0,
        product_id: 0,
        connect_type: ConnectType::Unknown,
        connected: false,
        battery_level: 0,
    };

    for field in parse_fields(data) {
        match field.number() {
            1 => {
                if let Some(value) = field.bytes() {
                    device.product_name = text(value);
                }
            }
            3 => {
                device.connect_type = match field.varint() {
                    Some(1) => ConnectType::Wired,
                    Some(2) => ConnectType::Receiver,
                    _ => ConnectType::Unknown,
                };
            }
            4 => {
                if let Some(value) = field.bytes() {
                    device.device_code = text(value);
                }
            }
            7 => {
                if let Some(value) = field.bytes() {
                    device.uid = text(value);
                }
            }
            8 => device.connected = field.varint().is_some_and(|value| value != 0),
            9 => {
                if let Some(value) = field.varint() {
                    device.battery_level = value.clamp(0, i64::from(u8::MAX)) as u8;
                }
            }
            11 => {
                if let Some(value) = field.varint() {
                    device.vendor_id = value.clamp(0, i64::from(u16::MAX)) as u16;
                }
            }
            12 => {
                if let Some(value) = field.varint() {
                    device.product_id = value.clamp(0, i64::from(u16::MAX)) as u16;
                }
            }
            _ => {}
        }
    }

    if device.uid.is_empty() {
        return None;
    }
    Some(device)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 一个 protobuf 字段：字段号加取值。
struct Field<'a> {
    number: u64,
    value: FieldValue<'a>,
}

/// protobuf 字段取值。
enum FieldValue<'a> {
    /// varint 取值。
    Varint(i64),
    /// 长度前缀内容。
    Bytes(&'a [u8]),
}

impl Field<'_> {
    /// 字段号。
    fn number(&self) -> u64 {
        self.number
    }

    /// 长度前缀内容；不是该线型时返回 None。
    fn bytes(&self) -> Option<&[u8]> {
        match self.value {
            FieldValue::Bytes(value) => Some(value),
            FieldValue::Varint(_) => None,
        }
    }

    /// varint 取值；不是该线型时返回 None。
    fn varint(&self) -> Option<i64> {
        match self.value {
            FieldValue::Varint(value) => Some(value),
            FieldValue::Bytes(_) => None,
        }
    }
}

/// 把一段 protobuf 拆成字段序列。
///
/// 只处理 varint 与长度前缀两类线型；其余线型（定长 32/64 位）跳过取值仍继续解析。
fn parse_fields(data: &[u8]) -> Vec<Field<'_>> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < data.len() {
        let Some((tag, used)) = read_varint(&data[index..]) else {
            break;
        };
        index += used;
        let number = tag >> 3;
        match tag & 7 {
            0 => {
                let Some((value, used)) = read_varint(&data[index..]) else {
                    break;
                };
                index += used;
                out.push(Field {
                    number,
                    value: FieldValue::Varint(value as i64),
                });
            }
            2 => {
                let Some((len, used)) = read_varint(&data[index..]) else {
                    break;
                };
                index += used;
                let end = (index + len as usize).min(data.len());
                out.push(Field {
                    number,
                    value: FieldValue::Bytes(&data[index..end]),
                });
                index = end;
            }
            5 => index += 4,
            1 => index += 8,
            _ => break,
        }
    }
    out
}

fn read_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0;
    for (index, byte) in data.iter().enumerate() {
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn device_bytes(
        product_name: &str,
        connect_type: u64,
        device_code: &str,
        uid: &str,
        connected: bool,
        battery: u64,
        vendor_id: u64,
        product_id: u64,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        push_bytes(&mut out, 1, product_name.as_bytes());
        push_varint_field(&mut out, 3, connect_type);
        push_bytes(&mut out, 4, device_code.as_bytes());
        push_bytes(&mut out, 7, uid.as_bytes());
        push_varint_field(&mut out, 8, u64::from(connected));
        push_varint_field(&mut out, 9, battery);
        push_varint_field(&mut out, 11, vendor_id);
        push_varint_field(&mut out, 12, product_id);
        out
    }

    fn push_varint_field(out: &mut Vec<u8>, number: u64, value: u64) {
        write_varint(out, number << 3);
        write_varint(out, value);
    }

    fn push_bytes(out: &mut Vec<u8>, number: u64, value: &[u8]) {
        write_varint(out, (number << 3) | 2);
        write_varint(out, value.len() as u64);
        out.extend_from_slice(value);
    }

    fn response(cmd: u64, devices: &[Vec<u8>]) -> Vec<u8> {
        let mut list = Vec::new();
        for device in devices {
            push_bytes(&mut list, 1, device);
        }
        let mut body = Vec::new();
        push_varint_field(&mut body, 1, cmd);
        push_bytes(&mut body, 5, &list);
        body
    }

    fn sample_device(level: u8) -> FlydigiDevice {
        FlydigiDevice {
            product_name: String::new(),
            device_code: String::new(),
            uid: "uid".to_string(),
            vendor_id: 0,
            product_id: 0,
            connect_type: ConnectType::Unknown,
            connected: true,
            battery_level: level,
        }
    }

    #[test]
    fn percent_steps_by_twenty() {
        let cases = [(0u8, 0u8), (1, 20), (2, 40), (3, 60), (4, 80)];
        for (level, expected) in cases {
            assert_eq!(
                sample_device(level).battery_percent(),
                expected,
                "level {level}"
            );
        }
    }

    #[test]
    fn percent_saturates_at_five_levels() {
        for level in [5u8, 6, 9, 255] {
            assert_eq!(sample_device(level).battery_percent(), 100, "level {level}");
        }
    }

    #[test]
    fn parses_a_receiver_device() {
        let dev = device_bytes(
            "Controller (Flydigi VADER3)",
            2,
            "f3p",
            "3891212C",
            true,
            4,
            0x045E,
            0x028E,
        );
        let devices = parse_connected_devices(&response(CMD_GET_CONNECTED_DEVICES, &[dev]));

        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.product_name, "Controller (Flydigi VADER3)");
        assert_eq!(device.device_code, "f3p");
        assert_eq!(device.uid, "3891212C");
        assert_eq!(device.connect_type, ConnectType::Receiver);
        assert!(device.connected);
        assert_eq!(device.battery_level, 4);
        assert_eq!(device.battery_percent(), 80);
        assert_eq!(device.vendor_id, 0x045E);
        assert_eq!(device.product_id, 0x028E);
    }

    #[test]
    fn parses_wired_connection() {
        let dev = device_bytes("Pad", 1, "f5", "AABB", true, 2, 0x37D7, 0x2502);
        let devices = parse_connected_devices(&response(CMD_GET_CONNECTED_DEVICES, &[dev]));

        assert_eq!(devices[0].connect_type, ConnectType::Wired);
        assert_eq!(devices[0].battery_percent(), 40);
    }

    #[test]
    fn missing_connect_type_is_unknown() {
        let dev = device_bytes("Pad", 9, "f5", "AABB", true, 1, 0x37D7, 0x2502);
        let devices = parse_connected_devices(&response(CMD_GET_CONNECTED_DEVICES, &[dev]));

        assert_eq!(devices[0].connect_type, ConnectType::Unknown);
    }

    #[test]
    fn missing_battery_field_defaults_to_zero() {
        let mut dev = Vec::new();
        push_bytes(&mut dev, 1, b"Pad");
        push_bytes(&mut dev, 7, b"AABB");
        push_varint_field(&mut dev, 8, 1);

        let devices = parse_connected_devices(&response(CMD_GET_CONNECTED_DEVICES, &[dev]));

        assert_eq!(devices[0].battery_level, 0);
        assert_eq!(devices[0].battery_percent(), 0);
    }

    #[test]
    fn device_without_uid_is_skipped() {
        let dev = device_bytes("Pad", 2, "f5", "", true, 3, 0x37D7, 0x2502);

        let devices = parse_connected_devices(&response(CMD_GET_CONNECTED_DEVICES, &[dev]));

        assert!(devices.is_empty());
    }

    #[test]
    fn empty_device_list_yields_nothing() {
        let devices = parse_connected_devices(&response(CMD_GET_CONNECTED_DEVICES, &[]));

        assert!(devices.is_empty());
    }

    #[test]
    fn multiple_devices_are_all_returned() {
        let first = device_bytes("A", 2, "f3p", "UID-A", true, 4, 0x045E, 0x028E);
        let second = device_bytes("B", 1, "f5", "UID-B", true, 1, 0x37D7, 0x2502);

        let devices =
            parse_connected_devices(&response(CMD_GET_CONNECTED_DEVICES, &[first, second]));

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].uid, "UID-A");
        assert_eq!(devices[1].uid, "UID-B");
    }

    #[test]
    fn strip_frame_rejects_missing_header() {
        // 长度足够但不以协议头开头：应报缺头，而不是报过短。
        let frame = b"NOT_A_PROTOCOL_HEADER_HERE";
        assert!(frames_have_no_header(frame));
    }

    fn frames_have_no_header(frame: &[u8]) -> bool {
        matches!(
            strip_frame(frame),
            Err(FlydigiError::Frame("missing protocol header"))
        )
    }

    #[test]
    fn strip_frame_rejects_short_input() {
        assert!(matches!(
            strip_frame(b"FDG_"),
            Err(FlydigiError::Frame("shorter than header"))
        ));
    }

    #[test]
    fn strip_frame_returns_declared_body() {
        let mut frame = Vec::new();
        frame.extend_from_slice(PROTOCOL_HEADER);
        frame.extend_from_slice(&3i32.to_le_bytes());
        frame.extend_from_slice(&[0x08, 0x81, 0x20]);

        let body = strip_frame(&frame).expect("帧可拆");
        assert_eq!(body, vec![0x08, 0x81, 0x20]);
    }
}
