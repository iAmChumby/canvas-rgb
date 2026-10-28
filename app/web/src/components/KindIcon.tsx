import {
  Cpu,
  Fan,
  GraphicsCard,
  Headphones,
  Keyboard,
  Lightbulb,
  Memory,
  Microphone,
  Mouse,
  type IconProps,
} from "@phosphor-icons/react";
import type { DeviceKind } from "@/proto/DeviceKind";

/** The Phosphor icon of a kind of device. A fan is a fan whatever board it hangs off. */
export function KindIcon({ kind, fan, ...rest }: { kind: DeviceKind | null; fan?: boolean } & IconProps) {
  if (fan) return <Fan {...rest} />;
  switch (kind) {
    case "dram":
      return <Memory {...rest} />;
    case "keyboard":
      return <Keyboard {...rest} />;
    case "mouse":
      return <Mouse {...rest} />;
    case "microphone":
      return <Microphone {...rest} />;
    case "headset":
      return <Headphones {...rest} />;
    case "gpu":
      return <GraphicsCard {...rest} />;
    case "motherboard":
      return <Cpu {...rest} />;
    default:
      return <Lightbulb {...rest} />;
  }
}
