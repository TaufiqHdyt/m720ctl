# LogiMgrDaemon — device communication layer (static RE notes)

Binary: `LogiMgrDaemon` 10.26.49, **arm64 slice** (all addresses below are arm64 VM addresses).
Method: `nm`/`strings`/`llvm-objdump --macho -d` on a `lipo -thin arm64` copy in /tmp. The daemon was never run.

Legend: **[C]** = confirmed from disassembly or literal strings. **[I]** = inferred (from names, public HID++ docs, or patterns).

---

## 1. Big picture

The daemon contains **two separate device stacks**, plus a DriverKit extension.

| Stack | Namespace / classes | Transport | Used for |
|---|---|---|---|
| **devio** (modern) | C++ `devio::` (~19k symbols), async futures (`devio::future/promise/continuable/executor`) | IOHIDManager (`IOHIDDeviceSetReport` / input-report callback); CoreBluetooth GATT ("Blepp") | All HID++ 2.0 devices (Unifying/Bolt/Nano receivers, BLE, USB) |
| **Akebono / AZ** (legacy) | ObjC `AZHIDNub`, `AZHIDDevice`, `AZHardwareRegistry`, `AKEventServiceClient`, `AKHPP200Feature*`, `K760`/`K811`/`M557`/`T631`/`T651` | IOKit user client into the dext (`LogitechAKEventService`) | Old Bluetooth devices whose HID++ 2.0 engine runs **inside the dext** |
| **dext** `com.logi.options.hidfilter` | DriverKit C++ | IOHIDInterface / IOUSBHostInterface filter | Legacy BT devices, the Unifying receiver interface shim, and virtual HID++ for 3 legacy USB mice |

Glue between the two worlds:
- ObjC `AKSamarkandDevice` wraps a `std::shared_ptr<devio::IDevice>` (`-initWithDevicePtr:`, `-internalDevicePtr`).
- `featureFromSamarkandDevice<devio::IFeatureXXXX>(AKSamarkandDevice*)` is a template that pulls a typed devio feature out of the device.
- `akebono::xTrace2a` is the ObjC-side logger. `devio::xTrace2a` is the devio-side logger.
- `devio::CDevio` is the façade/singleton. It offers `devices_async`, `config_get/set`, `swid`, `set_active`, `persistent_data_directory`, and `config_add_centurion_device`. Behind it sits `devio::DeviceManager`.

---

## 2. devio architecture

### 2.1 Layers (bottom → top)

1. **OS abstraction**: `devio::MacOSAbstractionLayer::*`. It is a mockable wrapper (`SetAbstractionLayerImpl`, `g_pImpl`) around these calls:
   - `IOHIDManagerCreate/SetDeviceMatching/RegisterDeviceMatchingCallback/RegisterDeviceRemovalCallback/CopyDevices/ScheduleWithRunLoop`
   - `IOHIDDeviceOpen/Close/SetReport/RegisterInputReportCallback/GetProperty/GetService`
   - `IORegistryEntryGetPath` and CFRunLoop functions

   The real implementation is `MacOSAbstractionLayerImpl`. For example, `IOHIDDeviceSetReport` is at 0x100327180, which jumps to `_IOHIDDeviceSetReport`. [C]
2. **Hardware buses and devices**. These are the `IIHardwareBus` / `IIHardwareDevice` interfaces.
   - `devio::MacOSBus`: the IOHIDManager enumerator.
     - Main methods: `createMatchingDict`, `deviceMatchingCallback`, `deviceRemovalCallback`, `onDeviceAdded`, `knownDevice`, `lookupPid`, `cachePid`/`savePidCache`.
     - Bluetooth methods: `startBluetoothMonitor`, `refreshBTDevices`, `onBLEDeviceInRange`.
     - Power: `registerForSystemPower`.
     - Also: `reports_pieces`, `secondary_busses`. [C]
   - `devio::MacOSDevice`: one `IOHIDDeviceRef`.
     - Lifecycle: `open`, `close`, `onDeviceThread`.
     - I/O: `write`, `deviceReportCallback` → `onInputReport`, `onBleppReport`.
     - Bluetooth: `checkDriverSupportedBTDevice`, `startBTExclusiveAccessRecoveryMode`.
     - Permissions: `checkHidPermission` (Input Monitoring TCC). [C]
   - `devio::MacOSBlepp` with ObjC `BleppDelegate`: a CoreBluetooth GATT HID++ channel (see §3.4). [C]
   - `devio::CombinerBus` / `CombinerDevice` (`add_piece`, `remove_piece`, `whole`) merge several HID interfaces ("pieces") into one logical device. `AdapterDevice` (`this_piece`, `expected_piece_mask`, `piece_max_write_size`) wraps one piece. [C]
   - `devio::UnifyingBus` / `UnifyingDevice`: receiver logic and the paired-device children. [C]
   - `devio::AccessoryBus` / `AccessoryDevice`: sub-devices behind a device (`Device::enable_accessory_bus`, `onBecomeAccessoryBus`). [C]
   - `devio::CenturionPacketLayer` / `CenturionTransport` / `CenturionDevice`: a third protocol named "Centurion++" (headset-class; feature IDs `Feature0000CenturionRoot` / `0001CenturionFeatureSet`). Out of scope. [C]
3. **`devio::DeviceManager`**:
   - Bus setup: `connectToHardwareBusses[Serialized|WithRepeat]`, `add_bus_async`/`add_bus_with_retry`, `lazyInitializeHardwareBusses`, `reenumerate_busses`.
   - Hot-plug: `onPlug`/`onUnplug`.
   - Device creation: `makeDevice(shared_ptr<IHardwareDevice>, BusType, int)` @0x1002d8134, `applyDeviceAdapters`, `device_is_bus`.
   - Filtering: `device_should_be_detected` @0x1002d0e2c. It switches on bus type. For one bus type, **PIDs 0xC500–0xC5FF** (receivers) are gated by config flags at `this+0x510..0x515`. [C for constants; I for flag meaning]
4. **`devio::Device`**: the logical HID++ device. It runs a per-device **write thread**:
   - Thread functions: `write_thread_proc`, `wt_process_request_once`, `wt_process_request_with_retry`, `wt_send_command`, `wt_do_fwcheck`, `wt_read_fwinfo`.
   - Request objects are `devio::DeviceRequest`.
   - Features: `features`, `feature_by_id(_async)`, `feature_by_index`, `load_feature`, `load_feature_root`, `load_feature_at_index`, `load_fwinfo_then_1D4B`.
   - Configuration: `set_configured_async` (cookie in feature 0x0020).
   - Feature cache: `perform_cache_read/write` writes XML files into `persistent_data_directory`.
5. **`devio::Feature`** (base class) and **`devio::FeatureXXXXName`** (one subclass per feature). Each subclass also implements an interface `devio::IFeatureXXXXName`.
   - `Feature::FeatureFactory::register_feature(type_info, id, FeatureCreator, FeatureProtocol)` registers features.
   - `Feature::create_feature(id, index, device, protocol)` builds them. Its log text is "…could not find registered type, creating generic feature". [C]
   - `devio::IFeatureForceInclusionXXXX` are dummy statics. The ObjC files construct them so the linker keeps each feature's creator. [C]

### 2.2 Enumerations (strings at strings.txt ~33105) [C]

- **FeatureProtocol**: `HID++ 1.0`, `HID++ 2.0`, `Centurion++`, `Unknown Feature Protocol`.
- **Wireless protocol types**:
  - `TwentySevenMhz`, `Quad`, `EQuad`, `DfuLite`, `EQuadLite`, `EQuadHighReportRate`, `EQuadGamepad`, `Gothard`, `DaggerRom`
  - `UnifyingV2`, `LightSpeed1`, `LightSpeed2`, `LightSpeed2LLPM`, `LightSpeed2CA`, `BlePro`
- **AKSamarkandDevice `-busTypeString`**: `Unifying`, `USB`, `BLE`, `UnifyingReady`, `unknown`.

### 2.3 Device enumeration [C]

- `MacOSBus::createMatchingDict` @0x1003393b0 builds this IOHIDManager matching dictionary:
  ```
  { VendorID: 0x046D,
    DeviceUsagePairs: [ {DeviceUsagePage:0xFF00, DeviceUsage:0x02},   // HID++ long collection (receivers/USB)
                        {DeviceUsagePage:0xFF43, DeviceUsage:0x00},   // BLE HID++ vendor page
                        ...extra pairs from a config vector... ] }
  ```
  The constants `mov w8,#0x46d`, `#0xff00`/`#0x2`, and `#0xff43`/`wzr` are at 0x1003393d0–0x1003394b8.
- `MacOSDevice::onInputReport` @0x10034d214 accepts only report IDs **0x10..0x12**. The check is `sub w8,w8,#0x10 ; cmp w8,#0x2`. [C]
- `IOHIDDeviceOpen` is called with **options = 0** (non-exclusive), so the device is not seized. [C]
- **Receivers** (`UnifyingBus::pairing_method` @0x10037a784) [C]:
  - PID **0xC548** → method 0 (`is_ble_pro_receiver`, i.e. **Bolt**).
  - PIDs **0xC52F** and **0xC534** → method 2 (`is_unifying_ready_receiver`, Nano-class).
  - Any other PID → method 1 (`is_unifying_receiver`, e.g. C52B/C532).
- **Receiver enumeration** uses HID++ 1.0 registers: `enumerate_devices[_attempt]`, `perform_arrival/connect/disconnect/removal`, and `set_receiver_connection_state` (fake-arrival request). See §3.3.
- **Bluetooth Classic and BLE HID** devices come through IOHIDManager like any other HID device. `MacOSBus::startBluetoothMonitor` and `MacOSDevice::setBluetoothDeviceObject` / `isPairedBluetoothDevice` track IOBluetooth state. Devices that the dext drives (§5) are flagged via `checkDriverSupportedBTDevice`. [C names; I semantics]

### 2.4 Software ID (swid) [C]

- `DeviceManager::set_swid(unsigned)` @0x1002c7bd4: if the value is outside 1..15, it picks **`rand()%5 + 10`** (0xA–0xE). The result is stored at `+0x42c`.
- `Device::swid()` reads `this+0x6f8`.
- The read path recognises other software's swids. Example log: "Report is a feature response from other SW using swid 0x%x". On a conflict it changes its own swid: "...so we will switch our swid, will ping".

---

## 3. HID++ framing

### 3.1 HID++ 2.0 request construction — `devio::Feature::createFeatureCommand` @0x100308494 [C]

```
100308614: ldr  x8, [x20]          ; out buffer (vector<Byte>, pre-sized to >=4)
100308618: mov  w9, #0xff10
10030861c: strh w9, [x8]           ; buf[0]=0x10 (short report), buf[1]=0xFF (device index placeholder)
100308620: ldr  w9, [x23, #0x88]   ; this->feature_index
100308624: strb w9, [x8, #0x2]     ; buf[2]=feature index
10030862c: ldr  x8, [x8, #0xb0]    ; device->swid()  (vtable+0xb0)
100308638: orr  w8, w0, w22, lsl #4; (function<<4) | swid
100308640: strb w8, [x9, #0x3]     ; buf[3]=function/swid
100308650: bl   vector::insert(...) ; append params
10030865c: cmp  x9, #0x8 ; b.lo     ; if total length >= 8 ...
100308664: mov  w9, #0x11 ; strb    ; ... buf[0]=0x11 (long report)
```

Resulting layout (one byte per cell):

```
 [0] reportID  0x10 short (7 B) | 0x11 long (20 B) | (0x12 very long accepted on RX)
 [1] devIndex  0xFF = direct/receiver; receiver children → slot+1 (see below)
 [2] featIdx   index from root 0x0000 lookup
 [3] (func<<4)|swid
 [4..] params
```

- The device index is patched per transport. `devio::UnifyingDevice::write` @0x100395fd0 does: `ldrb w8,[x0,#0x1c0] ; add w8,w8,#1 ; strb w8,[x9,#1]`, so **buf[1] = receiver slot + 1**. [C]
- `MacOSDevice::write` lambda (0x10034ae90–0x10034aec8) calls `SetReport(dev, kIOHIDReportTypeOutput(1), reportID=buf[0], buf+1, len-1)` when flag `+0x28a` is set. Otherwise it sends the whole buffer (0x10034b074). Log line: `-hid- 0x%x [W]: %s`. [C]

### 3.2 Response matching — `DeviceRequest::matches_response` @0x1002ff3c0 / `matchCmdHeader` @0x1002ff5ec [C]

- **Error reports**: `rsp[2]==0xFF` (HID++ 2.0 error) or `rsp[2]==0x8F` (HID++ 1.0 error).
  - Matched when `rsp[1]==req[1] && rsp[3]==req[2] && rsp[4]==req[3]`.
  - The HID++ 1.0 error has an extra case: `rsp[1]==0 && req[1]==0xFF`.
  - Error code is `rsp[5]`. [I: standard]
- **Normal responses**: `rsp[1..3] == req[1..3]`. An optional `vector<unsigned>` of extra byte offsets (at `+0x68`, relative to byte 4) must also match. This is `sendFeatureCommandWithMatch`.
- `Device::onReadHandler` classifies each report as one of:
  - "Hid++ 1.0 notification"
  - "HID++ 2.0 notification" (a report not matching the pending request; the swid nibble is checked with `and w26,w25,#0xf`)
  - "Hid++ command response"
  - "protocol check response from other SW"
  - "notif from an unknown feature at index"
  - It also has a "buffered response detected / brb workaround". [C strings]
- Retry and recovery strings in the write thread:
  - `wt_process_request_with_retry`: "sending ping", "on ping soft error ... retry", "Ignoring Hpp20Busy response".
  - Quarantine: "quarantining it for %d seconds".
  - Handle reset: "After %d no reponse, closing handles". [C]

### 3.3 HID++ 1.0 (receiver registers) — `devio::HPP10Command::*` [C]

Each command is built as a 7-byte short report. The constants are 32-bit immediates stored little-endian:

| Function | Bytes | Meaning (sub-ID / register) |
|---|---|---|
| `short_register_bit_operation[_async]` @0x10030f418 | `10 FF 81 <reg> 00 00 00`, then a SET | Read-modify-write of short register (e.g. reg 0x00 notification flags). `UnifyingBus::enable_receiver_notifications`, `set_software_present` |
| `read_receiver_connection_state` @0x10030cdf4 | `10 FF 81 02 00 00 00` | GET reg 0x02 (connected-device count) |
| `set_receiver_connection_state` @0x10030c25c | `10 FF 80 02 02 ..` | SET reg 0x02 = 0x02 → receiver replays 0x41 arrival notifications |
| `read_pairing_info` @0x100313690 | `10 FF 83 B5 <sub> ..` | GET_LONG reg 0xB5 (pairing info / device name; also `read_receiver_paired_device_name`) |
| `read_unique_identifier_async` | `10 FF 83 FB ..` | GET_LONG reg 0xFB |
| `read_receiver_firmware_info_async` | `10 FF 83 F4 <x>` | GET_LONG reg 0xF4 (fw info) [register meaning I] |
| `get_activity` | `10 FF 83 B3 ..` | GET_LONG reg 0xB3 (per-slot activity counters) |
| `open_lock` / `close_lock` | `10 FF 80 B2 01 ..` | SET reg 0xB2 = open pairing lock (Unifying) |
| `unpair` @0x1003110c4 | `10 FF 80 B2 03 <slot>` | SET reg 0xB2 = unpair slot |
| `enter_dfu` @0x100312814 | `10 <idx> 80 F0 ..` | SET reg 0xF0 (DFU control) |
| `start_session` @0x1003150e4 | `11 FF 82 F7 ..` | SET_LONG reg 0xF7 [purpose I] |
| `ble_pro_perform_discovery` @0x100311594 | `10 FF 80 C0 <a> <b>` | Bolt discovery reg 0xC0 |
| `ble_pro_pair` / `ble_pro_cancel_pairing` / `ble_pro_unpair` / `ble_pro_read_pairing_info` | reg 0xC1… | Bolt pairing (passkey flow) [I] |
| `mdf_get_info/enable/disable_features`, `read_dfu_control/set_dfu_control`, `set_passwd0`, `open_lock_LS2` | — | misc |

Receiver notifications decoded in `UnifyingBus::onRead` @0x100377670 (compares against `buf[2]`) [C]:

| Sub-ID | Meaning |
|---|---|
| 0x41 | device connect |
| 0x40 | device disconnect (unpaired) |
| 0x4A | lock change |
| 0x4D | Bolt passkey display |
| 0x4E | passkey entry |
| 0x4F / 0x52 | discovery advertise |
| 0x53 | discovery status |
| 0x54 | pairing status |
| 0x56 | DFU control timeout |

The handler also watches for other software writing register 0. Log: "detected write to receiver register 0, other software might have cleared software present and wireless notification bits" → `onRegisterZeroOverwritten` re-arms the notification bits.

### 3.4 BLE "Blepp" (HID++ over a proprietary GATT service) [C]

- `BleppDelegate` (a `CBCentralManager` / `CBPeripheral` delegate) discovers these services:
  - **`00010000-0000-1000-8000-011F2000046D`**, with characteristic `00010001-…-011F2000046D`.
  - `00050000-…`, with characteristic `00050001-…`.
  - Device Information service `180A`: `2A25` (serial) and `2A50` (PnP ID).
- RX path: `MacOSDevice::onBleppReport` @0x10034cdfc.
  - Rejects payloads of 0x13 bytes or more ("blepp report too long").
  - Prepends **`11 FF`** (`mov w8,#0xff11 ; sturh`) and copies the GATT payload after it.
  - So the GATT value is a HID++ long report **without report ID and device index** (≤18 bytes).
- TX path: `MacOSBlepp::dowrite` (log `-blepp- 0x%x [W]`) dispatches to a CB queue. [I: it strips the first two bytes symmetrically]
- `MacOSBlepp::switchBack` logs "switching back to HID++": a device in Blepp mode is moved back to the normal HID transport.

### 3.5 Root / FeatureSet lookup (HID++ 2.0) [C]

- `Device::load_feature_root` calls `Feature0000Root::TargetSw()` @0x1003ae708. This sends root function 1 with params `00 00 00` (ping / protocol-version). The `Ping(Byte)` variant @0x1003b0b2c puts the ping byte in params[2].
  - A HID++ 1.0 device answers with a 0x8F error, so `load_feature_root` concludes "not HPP 2.0 device" (NotHpp20). Otherwise it logs "set feature protocol to HID++ 2.0".
- `Feature0000Root::GetFeature(id)` @0x1003ada04 sends **function 0**, params `{id>>8, id&0xFF}` (`lsr w8,w24,#8 ; strb ; strb w24`). The response is unpacked into index, plus flags `obsolete` / `sw_hidden` / `eng_hidden` (bits 32/40/48) and version.
- `Feature0001FeatureSet::GetCount` uses function 0. `GetFeatureId(index)` @0x1003b6f9c uses **function 1** with params `{index}`.
- The feature table is cached as XML in the persistent data dir ("wrote feature cache to file %s"). It is invalidated when the firmware string from 0x0003 changes (`wt_do_fwcheck`).
- 0x0020 ConfigChange cookie: `set_configured_async` writes a random cookie. If the cookie is 0 after a reconnect, the device "lost settings" and the daemon re-applies its configuration.
- Function numbers spot-checked against the public spec (`mov w1,#n` before the send vcall):

  | Feature function | n |
  |---|---|
  | 1B04 GetCtrlIdInfo | 1 |
  | 1B04 SetCtrlIdReporting | 3 |
  | 2201 SetSensorDPI | 3 |
  | 2121 SetWheelMode | 2 |
  | 2110 SetRatchetControlMode | 1 |
  | 1814 SetCurrentHost | 1 |
  | 40A3 SetGlobalFnInversion | 1 |
  | 1004 GetStatus | 1 |
  | 2150 SetThumbwheelReporting | 2 |

---

## 4. Feature implementations (ObjC side)

### 4.1 How a device gets its features [C]

1. `devio::DeviceManager` produces a device.
2. `+[AKDeviceFactory newDeviceWithSharedPtr:]` @0x1000e9b58 picks a class with `+classByPid:` @0x1000e9a90:

   | PID | Class |
   |---|---|
   | 0xB010–0xB011 | `M557` |
   | 0xB00C | `T651` |
   | 0xB00D | `T631` |
   | 0xB316, 0xB318 | `K760` |
   | 0xB317 | `K811` |
   | anything else | **`AKSamarkandDevice`** → `-initWithDevicePtr:` |

   `+supportedPids` comes from `SHResourcesManager -loadSupportedPid` (resource file).
3. `-[AKSamarkandDevice obtainFeatures]` @0x100101ce0:
   - Takes `[[AKFeatureImplementationFactory sharedInstance] implementedFeatureIds]`, sorted.
   - Calls `featureWasObtained:` for each ID.
   - That calls `-newFeatureImplementation:withDevice:withSamarkandDevice:` @0x100110124, which:
     - looks up the class in the factory's `NSMutableDictionary` (`@(featureID)` → Class),
     - calls `+isFeatureSupportedForSamarkandDevice:`,
     - and if supported, calls `[[cls alloc] initWithDevice:]`.
4. **Registration** happens in static C++ initializers. Each `.mm` file has `__GLOBAL__sub_I_<File>.mm`. That initializer constructs `IFeatureForceInclusionXXXX` objects and calls `RegisterFeatureImplementation::RegisterFeatureImplementation(objc_class*, int featureID)` @0x1001101b0, which runs `-[AKFeatureImplementationFactory registerFeatureImplementationClass:forFeature:]`.
5. Each implementation follows the same lifecycle:
   - `initWithDevice:`, `setupDevice:pid_dict:context:`, `postSetupDevice:…`
   - `onConnected:`, `onDisconnected`, `onDeviceRemoved`
   - `defaultFeatureSettings:`
   - `AKCachedFeatureImplementation` provides write caching (`needSetupDeviceForKeys:inDictionary:`).
6. `+isFeatureSupportedForSamarkandDevice:`:
   - Generic classes: `featureFromSamarkandDevice<IFeatureXXXX>` is non-null **and** `[dev feature_by_id:0xXXXX]` is not obsolete/hidden (vtable+0x58).
   - **Per-device subclasses** also compare `[dev modelId]` against a constant.
7. **Pseudo feature IDs**: per-device subclasses register under synthetic IDs **0x6502–0x6545**, so several implementations can coexist for one real feature.

### 4.2 modelId format [C formula / I field meaning]

`devio::Feature0003FirmwareInfo::GetModelId` @0x1003c53e4 computes `orr w8, w9, w8, lsl #16`. That is **`modelId = (transport << 16) | firstModelPID`**, taken from 0x0003 GetDeviceInfo. If that is unavailable it falls back to the device PID.

Transport bitmask per the public 0x0003 spec: bit0 = BT, bit1 = BLE, bit2 = eQuad/Unifying, bit3 = USB.

| Prefix | Meaning |
|---|---|
| `1Bxxx` | BT Classic |
| `2Bxxx` | BLE-only (Mac/Bolt variants) |
| `4xxxx` | Unifying WPID |
| `6Bxxx` | BLE + Unifying |
| `8Cxxx` | USB |
| `EBxxx` | BLE + Unifying + USB |

### 4.3 Generic HID++ 2.0 features (class `AK<ID>FeatureImplementation`, devio class `devio::Feature<ID>…`) [C IDs; behaviour from method names]

| ID | devio class | Daemon behaviour (method names) |
|---|---|---|
| 0x0000 | Feature0000Root | GetFeature, Ping, TargetSw, GetProtocolVersion, TestPingWithMerge |
| 0x0001 | Feature0001FeatureSet | GetCount, GetFeatureId |
| 0x0003 | Feature0003FirmwareInfo | modelId / extendedModelId / unitId / serial / fw string → `AKDaemonAPI updateDevicePid:withModelId:…` and FW-update check |
| 0x0005 | Feature0005DeviceNameType | getDeviceName / getDeviceType (`createFilteredName:`) |
| 0x0020 | Feature0020ConfigChange | configuration cookie ("settings lost" detection) |
| 0x0021 | Feature0021CryptoIdentifier | GetIdentifier (used by Flow: `AKPDeviceImplementation readCryptoIdentifier`) |
| 0x1000 | Feature1000BatteryUnifiedLeveStatus | battery level/status → low-battery OSD |
| 0x1004 | Feature1004UnifiedBattery | GetCapabilities/GetStatus/events → OSD (`BatteryStateMachine1004`), analytics |
| 0x1300 | Feature1300LEDControl | take SW control of LEDs; enable/disable with timeout |
| 0x1814 | Feature1814ChangeHost | currentHost / nbHosts / switchToHost / cookies (Easy-Switch, Flow) |
| 0x1815 | Feature1815HostsInfos | host friendly name = computer name, OS version, Move/DeleteHost |
| 0x1981 | Feature1981KeyboardBacklight | backlight change events / duration |
| 0x1982 | Feature1982Backlight | backlight config/effects/info events |
| 0x1B04 | Feature1B04SpecialKeys | Reprog Controls v4: GetCount/GetCtrlIdInfo/Get/SetCtrlIdReporting. Daemon **diverts** buttons, handles rawXY (gesture button) and raw wheel, remaps, click analytics (`AK1B04ClickAnalytics`) |
| 0x1D4B | Feature1D4BWirelessStatus | reconnect events → re-apply config (`onWirelessStatusEvent`) |
| 0x2005 | Feature2005ButtonSwapCancel | Get/SetSwapMode (cancel left/right swap) |
| 0x2110 | Feature2110SmartShift | Get/SetRatchetControlMode (SmartShift threshold) |
| 0x2111 | Feature2111SmartShiftWithTunableTorque | same, plus torque |
| 0x2121 | Feature2121HiResWheel | Get/SetWheelMode (hi-res / invert / divert), ratchet switch events, wheel analytics; Photoshop-specific zoom mode |
| 0x2130 | Feature2130RatchetWheel | wheel mode (legacy) |
| 0x2150 | Feature2150Thumbwheel | GetThumbwheelInfo/Status/SetReporting; diverted rotation → gestures/app actions |
| 0x2201 | Feature2201AdjustableDPI | sensor DPI list / Get / SetSensorDPI (+ clamping); pointer speed → DPI |
| 0x2205 | Feature2205PointerMotionScaling | pointer scaling value |
| 0x2250 | Feature2250ActivityTracking | mouse-movement analytics |
| 0x2251 | Feature2251MouseWheelAnalytics | wheel/thumbwheel analytics |
| 0x40A0 / 0x40A2 / 0x40A3 | Feature40A{0,2,3}FnInversion | Fn-lock state; 40A3 is per-host (`onFLockChangeHostIndex:…`) |
| 0x4220 | Feature4220LockKeyState | Caps Lock state → OSD / menu-bar icon |
| 0x4530 | Feature4530DualPlatform | Get/SetPlatform (Mac/Win layout) |
| 0x4531 | Feature4531MultiPlatform | per-host platform descriptor; set Mac platform |
| 0x4600 | Feature4600Crown | Craft crown: GetInfo/Mode/SetMode, crown events → actions |
| 0x6501 | Feature6501Gestures | Gestures v2: enable/divert gestures, param get/set, finger/dxdy/trigger events (`AK650xGesture_*`) |

Legacy ObjC HID++ 2.0 path, used with the dext engine (`AKHPP200Feature*`, created by `+[AKHPP200FeatureFactory createFeature:]`): 0x0005, 0x1000, 0x1004, 0x1981, **0x1B00** (reprog keys v1) [I], **0x2001** (swap buttons) [I], 0x40A0, 0x40A2, **0x6110** (touchpad raw XY, for T651/T631) [I].

### 4.4 Per-device subclasses (pseudo-ID → modelId compared) [C]

The PID → product-name mapping in this table is mostly **[I]** (public PID knowledge). The class name is authoritative.

| Class | Pseudo-ID | Real feature | modelId check |
|---|---|---|---|
| AK1B04_K480 | 0x6509 | 1B04 | 0x1B33D |
| AK1B04_1B342 (K380?) | 0x650a | 1B04 | 0x1B342 |
| AK1B04_1B362 | 0x652d | 1B04 | 0x1B362 |
| AK1B04_M335BT / M336BT | 0x6505 / 0x6506 | 1B04 | 0x1B014 / 0x1B016 |
| AK1B04_M335 / M510 / M545 | 0x6504 / 0x6507 / 0x6508 | 1B04 | 0x44050 / 0x44051 / 0x44052 |
| AK1B04_ROMBasedMouse | 0x650f | 1B04 | 0x44055 |
| AK1B04_M275 / M275Lite | 0x650c / 0x650d | 1B04 | 0x44057 / 0x44058 |
| AK1B04_Flores | 0x6520 | 1B04 | 0x4406D |
| AK1B04_44094 | 0x652b | 1B04 | 0x44094 |
| AK1B04_M170 / M320 / MultiMouse | 0x650b / 0x650e / 0x6545 | 1B04 | (no constant; other test) |
| AK1B04_MXMaster | 0x6517 | 1B04 | 0x6B012 |
| AK1B04_MXAnywhere2 | 0x651b | 1B04 | 0x6B013 |
| AK1B04_M720 | 0x6511 | 1B04 | 0x6B015 |
| AK1B04_MXMasterRestage | 0x6519 | 1B04 | 0x6B017 |
| AK1B04_MXAnywhere2Restage | 0x651c | 1B04 | 0x6B018 |
| AK1B04_MXMaster2S | 0x6515 | 1B04 | 0x6B019 |
| AK1B04_MXAnywhere2S | 0x6516 | 1B04 | 0x6B01A |
| AK1B04_M585 | 0x6514 | 1B04 | 0x6B01B |
| **AK1B04_Cardea** (MX Ergo?) | 0x651f | 1B04 | 0x6B01D |
| AK1B04_MXMasterCR | 0x651a | 1B04 | 0x6B01E |
| AK1B04_MXAnywhere2CR | 0x651d | 1B04 | 0x6B01F |
| AK1B04_6B023 (MX Master 3?) | 0x6524 | 1B04 | 0x6B023 |
| AK1B04_MXAnywhere3 | 0x653c | 1B04 | 0x6B025 |
| AK1B04_6B34E (class name ≠ check) | 0x6542 | 1B04 | **0x6B027** |
| AK1B04_6b028 (MX Master 3 for Mac?) | 0x6525 | 1B04 | 0x2B028 |
| AK1B04_MXAnywhere3B | 0x653d | 1B04 | 0x2B02D |
| **AK1B04_HekaB** | 0x6543 | 1B04 | 0x2B02F |
| AK1B04_2B030 | 0x6530 | 1B04 | 0x2B030 |
| AK1B04_6B33B / 6B34C / 6B34D | 0x6510 / 0x6512 / 0x6513 | 1B04 | 0x6B33B / 0x6B34C / 0x6B34D |
| AK1B04_6B350 (Craft) | 0x6518 | 1B04 | 0x6B350 |
| AK1B04_6B353 | 0x6521 | 1B04 | 0x6B353 |
| AK1B04_6B359 / 6B35A / 6B35B / 6B361 | 0x6527 / 0x6523 / 0x6526 / 0x652c | 1B04 | same as name |
| AK1B04_6b364 | 0x6528 | 1B04 | **0x2B364** |
| AK1B04_2B363 / 2B365 / 2B369 / 2B36A / 2B36E | 0x652e / 0x652f / 0x6540 / 0x653f / 0x6541 | 1B04 | same as name |
| AK1B04_8C093 (USB M500s?) | 0x652a | 1B04 | 0x8C093 |
| AK1B04EB020 (MX Vertical?) | 0x6522 | 1B04 | 0xEB020 |
| AK2150_6B023 / AK2150_6B028 | 0x6531 / 0x6532 | 2150 | 0x6B023 / 0x2B028 |
| AK4600_CrownKeyboard | 0x651e | 4600 | 0x6B350 (Craft) |
| AK6501_MXMaster / Restage / CR | 0x6533 / 0x6534 / 0x6535 | 6501 | 0x6B012 / 0x6B017 / 0x6B01E |
| **AK6501_Kyoto** | 0x6536 | 6501 | 0x6B353 |
| AKMouseDeviceImplementation / AKTrackballDeviceImplementation | 0x6502 / 0x6503 | 0005 | device-type based |
| AKMouse_6b01d / 6B023 / 6b025 / 6b028 / 2B02D DeviceImplementation | 0x653e / 0x6538 / 0x653a / 0x6539 / 0x653b | — | 0x6B01D / 0x6B023 / 0x6B025 / 0x2B028 / 0x2B02D |
| AKPDeviceImplementation (**Logitech Flow**) | 0x6537 | 1814 | constants 0xEB020, 0xF4FEB, 0x14171, 0xF4FD8 (exclusion list?) [I] |

---

## 5. DriverKit extension `com.logi.options.hidfilter.dext`

Path: `…/LogiMgrDaemon.app/Contents/Library/SystemExtensions/com.logi.options.hidfilter.dext/` (inside the daemon bundle, not at the app top level).

- Version 1.0.5914, built with the DriverKit 20.2 SDK. Universal binary, ~970 KB, with symbols.
- The daemon installs it with `-[AZApplicationDelegate installDriver]` → `+[OSSystemExtensionRequest activationRequestForExtension:@"com.logi.options.hidfilter" queue:]`. Uninstall goes through `deactivationRequestForExtension`. [C]

### 5.1 Personalities (Info.plist) [C]

| Personality | IOUserClass | Provider | Match |
|---|---|---|---|
| BT Mouse – Fiji Mac / Win | `LogitechAKBTHIDDriver_Fiji` | IOHIDInterface | 046D:B011 / B010 |
| BT Mouse – SanJuan | `LogitechAKBTHIDDriver_T630` | IOHIDInterface | 046D:B00D |
| BT Trackpad – Edgepad | `LogitechAKBTHIDDriver_Edge` | IOHIDInterface | 046D:B00C |
| BT Keyboard – BabyPhoton US / EU | `LogitechAKBTHIDDriver_K760` | IOHIDInterface | 046D:B316 / B318 |
| BT Keyboard – Mexico City | `LogitechAKBTHIDDriver_K810` | IOHIDInterface | 046D:B317 |
| LogiUnifying | `LogiUnifyingUSBDevice` (+ `LogiHIDppShim`, child `LogiVirtualHIDDevice` 046D:**1000**) | IOUSBHostInterface | idProduct 0xC52B, interface 2 |
| USB Mouse – LS1 / M500 / Trackman Marble | `LogiUSBHIDDriver_{LS1,M500,TrackmanMarble}` (+ child `LogiVirtualHIDPP`) | IOUSBHostInterface | 0xC062 / 0xC069 / 0xC408, interface 0 |

- The BT personalities run as `AppleUserHIDEventService` and carry sub-services `LogitechAKEventService`, `AKHIDPP200Engine`, and user client `LogitechAKEventServiceUserClient`.
- Virtual devices use `IOUserServerName com.logi.options.virtual`.

### 5.2 What it does [C names; I purpose]

- **Legacy BT devices**: `LogitechAKBTHIDDriver__Common`
  - Parses reports (`handleReports`, `handleHidPlusPlusReport`) and re-dispatches keyboard, pointer and scroll events (`dispatchKeyboardEvent`, `dispatchRelativePointerEvent`).
  - Implements key remapping, button/wheel disabling and swapping, touch mode (Edge/T651 trackpad), caps-lock and battery feedback.
  - Runs a **full HID++ 2.0 engine inside the dext**: `AKHIDPP200Engine::HPP200_*`, with `AKHIDPPFeature` and `AKHIDPPReport` (deviceIndex, featureIndex, functionId, softwareIndex).
  - Events go to the daemon through `LogitechAKLLEventQueue` (`enqueueHIDPP20Report`, `enqueueHIDPP20FeaturePresence`, `enqueueBatteryLevel`, `enqueueGesture`, `enqueueTouchReport`, …).
- **Unifying receiver C52B, interface 2** (the HID++ interface): `LogiUnifyingUSBDevice` plus `LogiHIDppShim`.
  - Shim methods: `handleDJNotification`, `enumerateDevices`, `queueMessageToDevice`, `setSwitchedMode`/`setSwitchedState`, `startVirtualHIDDevice`.
  - It exposes a virtual HID device 046D:1000 (`LogiVirtualHIDDevice`).
  - [I] Purpose: arbitrate HID++ traffic and DJ notifications on the receiver, giving software a clean virtual HID++ endpoint.
- **Legacy non-HID++ USB mice** (LS1, M500, Trackman Marble): `LogiUSBHIDDriver_Base` + `LogiVirtualHIDPP`.
  - **Emulates a HID++ 2.0 device** in software, with classes `LogiFeature0000/0001/0003/0005/0020/1000/1B04/2121/2130/2205` and `LogiHIDPPInterface`/`LogiHIDPP10Interface`.
  - Translates commands into vendor USB requests (`LogiUSBVendorInterface::setSmartScrollEnabled`, `Interface::setMouseDpi`, `setHiRes`).
  - So devio can treat these mice as ordinary HID++ devices. [I, strongly suggested by names]

### 5.3 User-client protocol (daemon `AKEventServiceClient` ↔ dext `LogitechAKEventServiceUserClient`) [C]

- **Discovery**: `-[AZHIDNub start]` (0x100086b88) calls `_findHIDEventService:"LogitechAKEventService" inEntry:` on the HID device's registry entry. If the service is missing, it shows "required driver missing" plus a restart notification.
- **Open sequence**:
  1. `IOServiceOpen(service, mach_task_self, type 0)`, with retries and `usleep(25000)`.
  2. `_version` (selector 0).
  3. `open` (selector 1, scalar 1).
  4. `IODataQueueAllocateNotificationPort` and `IOConnectSetNotificationPort`.
  5. `IOConnectMapMemory` (a shared event queue).
  6. `startAsyncRequest` (selector 0x1F).
- **Dispatch table**: dext `ExternalMethod` @0x1000136b8 bounds-checks `selector <= 0x21` and indexes a 32-byte `IOUserClientMethodDispatch` array at 0x100024550. Decoded:

| Sel | Dext handler | In (scalars / struct) | Out | Daemon caller |
|---|---|---|---|---|
| 0x00 | s_getVersion | 0 | 1 scalar | `_version` |
| 0x01 | s_open | 1 | – | `_open` |
| 0x02 | s_close | 0 | – | `close` |
| 0x03 | notImpl | 0 | 1 | `clearElements` |
| 0x04 | notImpl | 1 | 1 | `hasReportID:` |
| 0x05 | s_queryHIDPP20Feature | 1 (feature id) | – | `queryForFeature:` |
| 0x06 | s_sendHIDPP20Feature | struct 0x20 | – | `sendHIDPP20Feature:withFunctionId:withPayload:payloadSz:` |
| 0x07 | s_setPointerResolution | 1 | – | `setPointerResolution:` |
| 0x08 | notImpl | 2 | – | `setPointerAxis:` |
| 0x09 | s_setPointerAcceleration | 1 | – | `setPointerAcceleration[Percent]:` |
| 0x0A | s_setWheelResolution | 1 | – | `setWheelResolution:` |
| 0x0B / 0x0C | notImpl | 1 | – | `setTiltResolution:` / `setExchangeCommandAndOption:` |
| 0x0D / 0x0E | notImpl | struct 0x28 / 0x18 | – | `postHIDEvent:` / `postHIDPointerEvent:` |
| 0x0F | s_postMouseButton | 2 | – | `postMouseButton:state:` |
| 0x10 / 0x11 | notImpl | 2 | – | double / triple click |
| 0x12 | s_postMouseButtonLock | 2 | – | `postMouseButtonLock:state:` |
| 0x13 | s_postMouseWheel | 4 | – | `postMouseWheel:horizontal:accelerate:scrollphase:` |
| 0x14–0x18 | notImpl | – | – | tilt / keyboard key / LEDs / sensor resolution |
| 0x19 | s_requestStates | 1 | – | `requestStates:` |
| 0x1A | s_setTouchMode | 1 | – | `setTouchMode:` |
| 0x1B | s_disableMouseButtons | 2 | – | `disableMouseButtons:buttonsToPrimary:` |
| 0x1C | s_disableMouseWheel | 1 | – | `disableMouseWheel:` |
| 0x1D | s_swapPrimaryMouseButton | 1 | – | `swapPrimaryMouseButton:` |
| 0x1E | s_invertMouseWheel | 1 | – | `invertMouseWheel:` |
| 0x1F | s_getEvent (**async**, completion) | 0 | 7 scalars | `startAsyncRequest` → `onAsyncComplete` |
| 0x20 | s_setPointerSpeedMultiplier | 1 | – | `setPointerSpeedMultiplier[Percent]:` |

- Many selectors are stubbed (`s_notImplemented`) in the DriverKit port; they are leftovers from the old kext API.
- HID++ results come back as event types `HPP200Request/Response/Error/Timeout/NoFeature/Broadcast/ASE/NotSupported` (`+[AKEventServiceClient _HIDPPResultString:]`) and are handled by `_handleHIDPPEvent:` and `_handleHIDPPFeatureQuery:`.
- **The modern devio path does not use this user client.** It talks HID++ directly through IOHIDManager (and through the virtual 046D:1000 device when the shim owns the receiver [I]).

---

## 6. Open questions / not verified

- Exact purpose of `HPP10Command::start_session` (reg 0xF7) and receiver register 0xF4.
- Whether devio opens the dext's `LogiVirtualHIDDevice` (046D:1000) when the shim is active; the extra usage-pair entries come from config and were not traced.
- Product names for many modelIds (HekaB 0x2B02F, Kyoto 0x6B353, Flores 0x4406D, 0x2B030, 0x6B33B, 0x6B34x, 0x2B36x…).
- `AKPDeviceImplementation` constants 0xF4FEB / 0xF4FD8 / 0x14171.
- Blepp TX framing (assumed: strip `11 FF`).
