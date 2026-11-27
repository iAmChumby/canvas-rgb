// The devices canvas-rgb drives itself, from each adapter's own `SUPPORTED` constant.
// `crates/canvas-adapters/tests/supported_devices.rs` reads this file and fails when the two
// lists drift, so the Help checklist cannot name a device the adapters do not know.

export interface SupportedDevice {
  name: string;
  vid: string;
  pid: string;
}

export const SUPPORTED_DEVICES: readonly SupportedDevice[] = [
  { name: "ROYUAN MonsGeek Keyboard", vid: "3151", pid: "4015" },
  { name: "GAMDIAS CHIONE Digital Display", vid: "1B80", pid: "B53C" },
  { name: "Attack Shark X6", vid: "1D57", pid: "FA60" },
  { name: "8BitDo Ultimate 2 Wireless (2.4G receiver, idle)", vid: "2DC8", pid: "6013" },
  { name: "8BitDo Ultimate 2 Wireless (XInput)", vid: "2DC8", pid: "310B" },
  { name: "8BitDo Ultimate 2 Wireless (DInput)", vid: "2DC8", pid: "6012" },
  { name: "8BitDo Ultimate 2 Wireless (Bluetooth)", vid: "2DC8", pid: "6012" },
];
