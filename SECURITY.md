# Security policy

## Supported versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | Yes       |
| older   | No        |

## Reporting a vulnerability

Report security problems privately. Do not open a public issue, a public pull request or a
discussion.

Use GitHub's private vulnerability reporting: on the repository's **Security** tab, choose
**Report a vulnerability**. The report is private to the repository's maintainers.

Include what you need to reproduce it: the version, the Windows version, the steps, and what an
attacker gains. Do not include real API tokens, and do not attach a bundle you have not read.

## Why this matters

canvas-rgb has parts that run with more rights than an ordinary app:

- **A Windows service runs as SYSTEM.** The installer creates `canvas-rgb-svc` without an account
  name, so it runs as the default account, LocalSystem (`installer/lib.ps1`, `Get-ServiceCreateArgs`).
  The service talks to the daemon through the named pipe `\\.\pipe\canvas-rgb-svc`
  (`crates/canvas-proto/src/pipe.rs`).
- **Fan control uses a driver.** Fan control needs the PawnIO driver, which the installer does not
  ship. See [README.md](README.md).
- **A local API listens on 127.0.0.1:9430.** Every route except the status and the live preview
  needs an API token. The token is in the user's `%APPDATA%\canvas-rgb` folder, in a file named
  `api-token` (`crates/canvas-proto/src/consts.rs`).

## In scope

- Anything that lets a process or a user gain rights they should not have through the service, the
  named pipe or the installer and uninstaller.
- Anything that lets a process other than the user reach the API or its token, or that lets a
  request from a web page or another machine reach it.
- Anything that lets an effect file reach outside its sandbox. Effects run in the embedded runtime
  described in [docs/effects.md](docs/effects.md), which lists no network, DOM or storage calls.
- Anything that lets the fan control leave a fan in an unsafe state, or that keeps a fan from going
  back to BIOS control when the service stops.
- Anything in canvas-rgb's own code that writes outside the folders it uses, or that runs a file it
  downloaded without checking it.

## Out of scope

- Bugs in other software: OpenRGB, PawnIO, SignalRGB, iCUE and the vendor apps. Report those to
  their makers.
- The Windows SmartScreen warning on the unsigned installer. [README.md](README.md) says SmartScreen
  warns on first run. A problem with the installer's checks, such as the OpenRGB SHA-256 check, is in scope.
- Problems that need the attacker to already have administrator rights on the machine.
