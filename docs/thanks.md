# Built on other people's work

I could not have built canvas-rgb from scratch. Most of what makes it work was figured out by
other people first, and they shared it. This is who, and what each one made possible.

**OpenRGB** ([codeberg.org/OpenRGB/OpenRGB](https://codeberg.org/OpenRGB/OpenRGB)). The hard
part of PC lighting is talking to the hardware: motherboard ARGB controllers, RAM over SMBus,
a different chip and protocol for every vendor. OpenRGB's contributors have spent years reverse
engineering all of it. canvas-rgb runs their official release as a separate program and sends it
colours; it drives my fan rings, my RAM and my mic. Without OpenRGB this project would only light
up a keyboard.

**Hypercolor** ([github.com/hyperb1iss/hypercolor](https://github.com/hyperb1iss/hypercolor)).
A Rust lighting engine that showed how to speak OpenRGB's protocol from Rust and how to reach the
fan chip through PawnIO. My protocol code started from theirs (Apache-2.0, credited in NOTICE).

**PawnIO** ([github.com/namazso/PawnIO](https://github.com/namazso/PawnIO)). A signed driver that
lets user code reach the Super I/O chip safely. The fan control runs on it.

**nct6687d** ([github.com/Fred78290/nct6687d](https://github.com/Fred78290/nct6687d)) and
**LibreHardwareMonitor** ([github.com/LibreHardwareMonitor/LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)).
The register map of the fan chip on my board came from their drivers.

**ATKsoft** ([github.com/grivia0/ATKsoft](https://github.com/grivia0/ATKsoft)). The Attack Shark
X6 support exists because this project had already worked out the mouse's read-back handshake
and report layouts. **prodmutant**, **AlternativeLua**, **DavBfr** and **libratbag** did the same
for the X3 and X11, which is how I knew the bytes were right.

**ascendedent/8bitdo-Linux-Software** ([github.com/ascendedent/8bitdo-Linux-Software](https://github.com/ascendedent/8bitdo-Linux-Software)).
The only public map of the 8BitDo Ultimate 2's configuration channel. The profile read in
canvas-rgb uses the request layout they documented, and it worked on my controller the first
time. That is why I can remap my paddles without the 8BitDo app.

**SDL** ([github.com/libsdl-org/SDL](https://github.com/libsdl-org/SDL)). Its 8BitDo driver
documents the controller's input reports.

**SignalRGB** ([signalrgb.com](https://signalrgb.com)). canvas-rgb reads effects in SignalRGB's
HTML format, so the effects people already wrote for it can run here. Their effects belong to
their authors and are not part of this repo.

If your work belongs on this list and is missing, open an issue and I will add it.
