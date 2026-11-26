# Privacy: what leaves the PC

Work in progress — the full, evidence-backed table is being added on branch `feat/581-privacy`.

canvas-rgb sends an update check to GitHub (github.com) when it starts and at most once a day, with a second GitHub request only when the experimental channel is on; when you choose to install, it downloads the installer from GitHub. The installer downloads OpenRGB from codeberg.org while you install the app. Everything else stays on this PC: the editor and CLI talk to the local daemon on 127.0.0.1, the daemon talks to a local OpenRGB server, and the daemon and service use a local named pipe. Clicking a link opens your browser; canvas-rgb fetches nothing else.
