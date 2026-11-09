# Reverse engineering: LogiMgrDaemon (Logi Options daemon, macOS)

Static analysis only. Target is software already installed on this machine.
Nothing under `/Applications` was modified and the daemon was **not** executed.

- Bundle: `/Applications/Logi Options.app/Contents/Support/LogiMgrDaemon.app`
- Executable: `Contents/MacOS/LogiMgrDaemon` — Mach-O universal (x86_64 + arm64), 47 MB,
  **not stripped** (~42k local symbols: Objective-C + C++).
- Version: `10.26.49` (`Logi Options Daemon 10.26-49`), signed `Developer ID Application: Logitech Inc. (QED4VVPZWA)`, hardened runtime.
- Build SDK macOS 12.1; min macOS 10.15 (x86_64) / 11.0 (arm64). `LSUIElement` (no Dock icon, background agent).

## Entitlements
```
com.apple.security.app-sandbox            = false      (NOT sandboxed)
com.apple.security.automation.apple-events= true       (drives other apps via AppleEvents)
com.apple.developer.system-extension.install = true    (installs the DriverKit dext)
com.apple.developer.driverkit.userclient-access = [ com.logi.options.hidfilter ]
com.apple.developer.team-identifier       = QED4VVPZWA
```
Info.plist also declares: `NSAppleEventsUsageDescription`, `NSBluetoothAlwaysUsageDescription`
(CoreBluetooth), `NSAppleScriptEnabled = Yes` (scriptable), analytics group `com.logitech.OptionsSuite`.

---

## 1. High-level architecture

Mixed Objective-C (AppKit UI shell, menu-bar/OSD, glue) over a large portable C++ core.
Dominant C++ namespaces (by symbol count):

| Namespace | Role |
|---|---|
| `devio::` | **Device I/O / HID++ engine.** Enumerates receivers & BLE devices, frames HID++ reports, implements each HID++ 2.0 feature as `devio::IFeature<ID><Name>`. Async futures/promises model (`devio::future`, `promise`, `executor`, `continuable`). |
| `logitech::` / `Logitech::` | Higher-level device/app logic, Flow, `logitech::marconi::NodeStoreClient` (cloud rendezvous), `logi::sso::lip` (sign-in keystore). |
| `PluginManager::` | Local WebSocket server for browser/SDK plugins (`PluginManager::PluginManagerImpl`). |
| `GestureEngine*`, `*Recognizer`, `TouchFrame::` | Trackpad/mouse gesture recognition. |
| `thekogans::util::` | Utility lib (GUIDs, etc. — plugins are keyed by `thekogans::util::GUID`). |
| `web::`, `pplx::`, `cpr::`, `Concurrency::` | Microsoft **cpprestsdk** + **cpr** HTTP clients (telemetry, backup, Flow rendezvous). |
| `websocketpp::`, `asio::`, `boost::` | WebSocket/ASIO transport for the plugin server. |
| `nlohmann::`, `rapidjson::`, `pugi::`, `TBXML` | JSON / XML parsing (manifests, config, messages). |

Statically links **OpenSSL 1.0.2u** (cipher-name tables show up in strings; curl/NTLM/SSL handshake strings present).

ObjC side: ~546 classes, prefix `AK*` (Akebono — Logitech's internal codename for Options) and `AZ*`.
`AZApplicationDelegate` is the app delegate; `AKDaemonAPI` is the central service object.

---

## 2. Device layer (HID++)

**Two stacks exist.** The **modern** one is C++ `devio::` (async/future-based) on **IOHIDManager** +
CoreBluetooth; reached via `CDevio` → `DeviceManager` → `Device`/`Feature`. The **legacy** one is ObjC
`AZHIDNub` / `AKEventServiceClient` talking to the DriverKit dext (§4). `AKSamarkandDevice` is an ObjC
wrapper around a `shared_ptr<devio::IDevice>`.

### Transports / enumeration (confirmed)
- **USB** — matches **vendor 0x046D** with HID usage pairs **(0xFF00, 0x02)** and **(0xFF43, 0x00)**; opens
  devices **non-exclusively**; only accepts **report IDs 0x10–0x12**. Receiver type by PID:
  **0xC548 = Bolt**, **0xC52F/0xC534 = Nano**, anything else = **Unifying**
  (`devio::UnifyingBus::enumerate_devices`, `IReceiver`, `BleProPairing`).
- **Bluetooth LE** — `BleppDelegate` (CoreBluetooth). GATT service **`00010000-0000-1000-8000-011F2000046D`**
  (characteristic `…0001`); inbound GATT payloads get **`11 FF`** prepended. Log tag `-blepp-`.

### HID++ framing (confirmed from disassembly)
`Feature::createFeatureCommand` @`0x100308494` builds the report:
```
10 FF <featureIndex> <(function<<4)|swID> <params…>
```
and switches report ID **0x10 → 0x11** once length reaches 8 bytes. Details:
- `UnifyingDevice::write` overwrites byte 1 with **(receiver slot + 1)** (device index on the receiver).
- `MacOSDevice::write` emits via `IOHIDDeviceSetReport(Output, reportID=buf[0], buf+1, len-1)`.
- **swID** = `rand()%5 + 10` when unset; the daemon re-rolls it if another program collides.
- `DeviceRequest::matches_response` matches bytes 1–3 against the request; byte 2 == **0xFF** flags a
  HID++ 2.0 error, **0x8F** a HID++ 1.0 error.
- **Feature resolution:** root `0x0000` fn 0 with params `{id>>8, id&0xFF}` → feature index; feature set
  `0x0001` fn 0 = count, fn 1 = ID at index. **1.0 vs 2.0** decided by a root *ping* (fn 1); a `0x8F`
  reply ⇒ HID++ 1.0.

HID++ 1.0 receiver side lives in `devio::HPP10Command::*` — register commands `0x02/0xB2/0xB3/0xB5/0xFB/
0xF0/0xC0` and notification codes `0x40/0x41/0x4A/0x4D–0x56` were decoded (e.g.
`read_receiver_connection_state` → `connectedDevices=%d remainingSlots=%d`).

### HID++ 2.0 features implemented (from `devio::IFeature<ID>` + `AK<ID>FeatureImplementation`)

| ID | Name (from binary) | ID | Name |
|---|---|---|---|
| 0000 | Root | 1D4B | WirelessStatus |
| 0001 | FeatureSet | 1B04 | **SpecialKeys / reprogrammable controls (reprog v2/v4/v5)** |
| 0003 | FirmwareInfo (fw ver, **serial number**) | 2005 | ButtonSwapCancel |
| 0005 | DeviceNameType | 2110 | SmartShift |
| 0020 | ConfigChange (cookie) | 2111 | SmartShiftWithTunableTorque |
| 0021 | CryptoIdentifier | 2121 | HiResWheel |
| 1000 | BatteryUnifiedLevelStatus | 2130 | RatchetWheel |
| 1004 | UnifiedBattery | 2150 | Thumbwheel |
| 1300 | LEDControl | 2201 | AdjustableDPI |
| 1814 | ChangeHost | 2205 | PointerMotionScaling |
| 1815 | HostsInfos | 2250 | ActivityTracking |
| 1981 | KeyboardBacklight | 2251 | MouseWheelAnalytics |
| 1982 | Backlight | 40A0/40A2/40A3 | FnInversion |
| 4220 | LockKeyState | 4530 | DualPlatform |
| 4531 | MultiPlatform | 4600 | Crown (MX Master crown) |
| 6501 | Gestures | | |

Also an `AKHPP200Feature*` family and HID++ **1.0** device objects (`AKHPP200Feature1B00`, `2001`, `6110`…)
for older devices.

### Device object construction (confirmed)
- `AKDeviceFactory classByPid:` routes **old Bluetooth PIDs** (B00C/B00D/B010–B011/B316–B318) to dedicated
  classes (`T651`/`T631`/`M557`/`K760`/`K811`) and **everything else to `AKSamarkandDevice`** (the unified
  model, wrapping `shared_ptr<devio::IDevice>`).
- `AK<ID>FeatureImplementation` classes self-register via static initializers
  (`RegisterFeatureImplementation`) into `AKFeatureImplementationFactory`. `obtainFeatures` keeps a class
  only if its `+isFeatureSupportedForSamarkandDevice:` passes.
- **`modelId = (transport << 16) | PID`** (confirmed in `GetModelId` @`0x1003c53e4`). Per-device
  `AK1B04_<codename>Implementation` subclasses register under **made-up feature IDs 0x6502–0x6545** and
  additionally gate on `modelId`. Decoded model IDs include MXMaster=`0x6B012`, Cardea=`0x6B01D`,
  Kyoto=`0x6B353`, Flores=`0x4406D`, HekaB=`0x2B02F`, Craft crown=`0x6B350`. (Several codenames — HekaB,
  Kyoto, Flores, some 6B34x/2B36x — remain unmapped to retail names.)

---

## 3. Plugin WebSocket server (browser + SDK plugins)

`PluginManager::PluginManagerImpl` runs a **websocketpp/asio** server (`StartServer()` / `Run()` /
`StopServer()`), bound to `127.0.0.1:10134` (`0x2796`, hard-coded in
`PluginManagerImpl::Run`; listen retry loop up to 20 attempts). Started **on demand** via
`-[AZApplicationDelegate(PluginManager) webSocketsServerRequired:]` on its own thread — only when a
**"HOSTED"/"SDK"** plugin is active (e.g. Lightroom), so the port is usually not listening. The ObjC
`ChromePluginImplementation` / `FirefoxPluginImplementation` / `SafariPluginImplementation` are **in-daemon
foreground-app detectors (AX/AppleScript), not WebSocket clients**.

### Protocol
JSON over RapidJSON. Message classes `JSONMessage` → `PluginMessage` / `RegistrationMessage` /
`ToolUpdateMessage`; `on_message` dispatches to `HandleRegistrationMessage` / `HandleToolUpdateMessage`.
First frame carries "app data in handshake". Known JSON fields (from parse-error strings): `channel`,
`device_id`, `unit_id`, `origin` (numbers), `enable` (bool), `command`. Plugins register with a
`PluginGUID` / `sessionGUID` (`thekogans::util::GUID`), checked against a whitelist
(`isWhiteListedPluginGUID`); duplicate registration is detected. Server
dispatches device events to plugins: `Feature1b04Event` (button), `Feature2150Event` (thumbwheel),
`Feature4600Event` (crown), `HandleTurn`, `OnPressTurn`, `ToolChanged`, `SetToolOption`, `GetSpeedFactor`,
`EnablePlugin`. Example config payloads emitted:
```
[{"name":"%s","id":%d,"min":%d,"max":%d,"gesture":%d,"type":"bool","chart":%d}]
```
Plugin manifests: `~/Library/.../Logitech Options/PluginCustomizations/<GUID>/Manifest/tools.json`
(+ `defaults.json`), signature-verified (`.json.sig` / `.json.sigx`; "Cannot verify manifest signature of a
NULL plugin").

---

## 4. DriverKit system extension `com.logi.options.hidfilter.dext`

`Contents/Library/SystemExtensions/com.logi.options.hidfilter.dext` — DriverKit userspace driver,
version `1.0.5914`, SDK `driverkit.macosx20.2`. Universal x86_64+arm64.

- **11 IOKit personalities**, all `VendorID 1133` (0x46D Logitech), matching `IOHIDInterface`. Device driver
  classes: BT keyboards `LogitechAKBTHIDDriver_{K760,K810,Fiji,T630,Edge}`, USB mice
  `LogiUnifyingUSBDevice`, `LogiUSBHIDDriver_{LS1,M500,TrackmanMarble}`. (These are **legacy** devices that
  need a user-space HID driver; modern devices are driven by `devio` in the daemon over raw HID.)
- Service/userclient classes: `LogitechAKEventService`, **`LogitechAKEventServiceUserClient`**
  (`IOUserUserClient`), `AKHIDPP200Engine`, `LogiHIDppShim`, `AKHIDPPReport` / `AKHIDPPFeature`.
- **What it drives** (confirmed): the **legacy** devices only — old Bluetooth keyboards (Fiji, SanJuan,
  Edge, K760, K810) via an HID++ 2.0 engine running *inside* the dext; it also attaches to **interface 2 of
  the 0xC52B Unifying receiver** (`LogiHIDppShim`) and publishes a **virtual HID device 046D:1000**; and it
  emulates HID++ 2.0 for three old USB mice (LS1, M500, Trackman Marble). The **modern `devio` path does not
  use this user client** — it goes straight through IOHIDManager.
- The daemon's entitlement `driverkit.userclient-access = com.logi.options.hidfilter` lets it
  `IOServiceOpen` this user client. The **33-entry `ExternalMethod` table** (selectors 0x00–0x20) is the
  daemon→dext API; e.g. 5 = `queryHIDPP20Feature`, 6 = `sendHIDPP20Feature` (0x20-byte struct),
  0x1F = async `getEvent`. Dispatch functions (`s_*`):

```
s_open / s_close / s_getVersion / s_getEvent / s_requestStates / s_setTouchMode
s_queryHIDPP20Feature / s_sendHIDPP20Feature      <- HID++ 2.0 passthrough
s_postMouseButton / s_postMouseButtonLock / s_postMouseWheel
s_setWheelResolution / s_invertMouseWheel / s_disableMouseWheel / s_disableMouseButtons
s_setPointerResolution / s_setPointerAcceleration / s_setPointerSpeedMultiplier
s_swapPrimaryMouseButton / s_notImplemented
```
So the dext both (a) injects synthetic mouse button/wheel/pointer events and remaps buttons for those
legacy devices, and (b) tunnels HID++ 2.0 feature queries. `AKHIDPPReport::initWithFeature(feature, b1, b2,
bytes*, len)` builds the report struct (disasm writes feature/func bytes at offsets 0x8–0xB then `memcpy`s
the payload) — a structured HID++ report object. On the daemon side the matching ObjC glue is
`AKEventServiceClient` / `AKEventServiceConnection` and `IOHIDSystemUtils`.

---

## 5. IPC, persistence, cloud

- **Daemon → Options UI (request/response): Cocoa Distributed Objects.** `-[AKDaemonAPI installServer]`
  creates an `NSConnection`, sets the shared `AKDaemonAPI` as root object, and `registerName:`s it
  under a `com.logitech.manager.daemon-…` name. The UI uses it for device/profile config
  (e.g. `addProfileForPid:unitId:…`) and account/backup requests.
- **Daemon → UI/helpers (fire-and-forget): distributed notifications** named `com.logitech.manager.*` /
  `com.logitech.options.daemon.*` (e.g. `…daemon.device_notification`, `…application.restart.notification`,
  `…newsoftwareinstall.notification`, `…options.plugin_install_notification`,
  `…USBChargingFlowDisabled.notification`, `com.logitech.logimgr.updater_notification`,
  `com.logitech.PreferenceManager.Update.Notification`). `AKAnalytics` listens on both daemon- and UI-side
  channels. Central object `AKDaemonAPI` also drives `_configureAllDevices:`, `processReceiver:`,
  `_loadAllPaired{Bluetooth,Devio}Devices`, `checkLogiMgrUpdates`. Updater is a separate app
  `LogiMgrUpdater.app` / `Logi Bolt Installer`.
- **Daemon → IOKit driver (not UI):** `AKEventServiceClient` / `AKEventServiceConnection` do `IOServiceOpen`
  on `com.logi.options.hidfilter` with a Mach-port event queue (`_eventQueueMachPort`, `handleMachMessage:`)
  for HID injection/query (`postHIDEvent:`, `postMouseWheel:…`, `postKeyboardKey:usage:state:`,
  `sendHIDPP20Feature:withFunctionId:withPayload:payloadSz:`). `AKMultiPlatformInterface` is **not IPC** — it
  wraps HID++ feature 0x4531 (device-side host switching).
- **Preferences / state files:**
  `/Library/Preferences/com.logitech.manager.generalsettings.plist` (+ Managed Preferences variant),
  `~/Library/Preferences/com.logitech.manager.{devinfo,setting.*}.plist`,
  `/Users/Shared/.logishrd/com.logitech.OptionsSuite.plist`,
  `/Users/shared/.logishrd/com.logitech.Options.{eventCache,notificationHistory}.plist`.
- **Sign-in (SSO) + cloud settings backup:** `AKSSOBackend` / `AKSSOBackups` / `logi::sso::lip::key_store_api`.
  Endpoints in-binary: `https://accounts.logi.com`, `https://optionsbackup.logitech.com`,
  `https://nodestore.logitech.io/{store,fetch,erase}/…`. `AKDaemonAPI` exposes a full SSO surface
  (SignIn{Email,Google,Facebook,WebSSO}, SignUp, backup Store/Restore/List/Delete per device serial).
  Per-device settings are backed up keyed by **device serial** (`Feature0003FirmwareInfo::GetDeviceSerialNumber`).
- **Telemetry:** `https://logging.dhg.myharmony.com/1.0/logger/submitUsageData?Logitech-API-Key=…`
  (`LOGIAnalytics.framework`, `AKAnalytics*`).
- **Logi Flow** (cursor/clipboard across machines) — built on Logitech's **`marconi`** net library
  (`logitech::marconi`, over the `thekogans` stream framework). `logitech::flow::Flow::StartMarconiServer`
  → `marconi::DiscoveryMgr::Start`.
  - **Ports:** TCP `59866` + UDP `59867`, config-driven (`FlowConnectionPort`, consecutive TCP/UDP pair).
  - **Discovery:** UDP broadcast beacons (`BroadcastDiscovery`, `BeaconPacket`, `AutoDiscoveryJob`) plus
    `NodeStoreDiscovery` via `marconi::NodeStoreClient::GetPeerAddresses` (cpprestsdk → nodestore.logitech.io).
  - **Transport encrypted:** DTLS-over-UDP (`SecureUDPSocket` / `Client`/`ServerSecureUDPSocket`,
    `marconi::DecryptFilter`, OpenSSL).
  - **Payloads:** clipboard sync (`MESSAGE_ID_CLIPBOARD_GET/SET`, `FlowPasteboardSessionManager`), file copy
    (`CopyFileTransactionServer`), host switch (`MESSAGE_ID_HOST_SWITCH[_MAGNET_CHANGE]`) tied to HID++
    `1814 ChangeHost` / `1815 HostsInfos` and edge "switch zones". Log tag `-flowapi-`.

---

## 6. Notable extras
- **Per-app action engine:** hundreds of `AK*Action` classes + app-specific gesture handlers for Final Cut
  Pro, Premiere, Photoshop, Lightroom, Word, Excel, PowerPoint, Safari/Chrome/Firefox, WeChat emoji, etc.,
  driven through AppleEvents/AppleScript (`AppleScriptRunner`) and synthetic keystrokes (`AKKeystrokeAction`).
- **Gesture engine:** `GestureEngineImpl` + recognizers (NFingerSwipe, TwoFingerZoom, Flick, ThreeFingerDrag,
  FourFingerSpreadPinch…) feeding `TouchFrame`.
- Needs **Input Monitoring / Accessibility** TCC permissions (`IOHIDCheckAccess`,
  "our process is not trusted…", InputMonitoringImage.png) to listen for buttons and move the cursor.

## Scope / caveats
Everything above is from the shipped binaries (symbols, strings, Info.plists, and spot ARM64 disassembly of
`StartServer` and `AKHIDPPReport::initWithFeature`). Feature-ID→name and the dext method
table are **confirmed** from in-binary names; byte-level HID++ offsets and the plugin JSON schema are
**partially inferred** from format strings and would need runtime tracing to pin down exactly. The daemon
was not run and no Logitech endpoints were contacted.
