# <View group>: GPUI parity

Issue: #<N> | React pinned at: `<short SHA>` (`git log -1 --format=%h -- <files>` when this file was written)
React files: `app/web/src/views/<View>.tsx`, … | GPUI files: `app/gpui/src/views/<view>.rs`, …

An item is ticked only with how it was checked: a test name, a screenshot pair, or a command and what it showed.
An unchecked item stays `[ ]`. "Looks fine" is not a check.

## Controls
<!-- One line per control: what it is, what it does, which API call it makes. -->
- [ ] <control>: <behaviour> — checked by <test | pair | command>

## States
- [ ] Loading — checked by
- [ ] Empty — checked by
- [ ] Error (each API error the React view handles) — checked by
- [ ] Daemon down — checked by
- [ ] <view-specific state> — checked by

## Breakpoints
<!-- Every breakpoint the React view's CSS uses, from 640 to 1500, and what changes at each. -->
- [ ] <width>px: <what changes> — checked by pair at <size>

## Keyboard
- [ ] <key or path>: <result> — checked by

## Motion
- [ ] <animation>: <duration and curve from motion.css> — reduced-motion form: <…> — checked by

## API calls
- [ ] `<METHOD> <route>`: <when it fires>, <error handling> — checked by

## Drift check
Run before marking done: `git log <pinned SHA>..HEAD --oneline -- <React files>`.
- Commits since the pin: <list, or "none">
- Each folded in: [ ] <sha> <what changed> — checked by

## Screenshot pairs
Taken by `tools/ui-capture/` against the daemon on the fakes. 6 sizes × 2 themes = 12 pairs.

| Size | Dark (React / GPUI) | Light (React / GPUI) | Pixel diff (changed / total) |
|---|---|---|---|
| 1920x1040 | | | |
| 1440x900 | | | |
| 1280x860 | | | |
| 1024x768 | | | |
| 960x1032 | | | |
| 390x844 | | | |

## Reviewer verdict
Adversarial reviewer, date, what it compared, every mismatch it found and how each was fixed, and its final verdict
(PASS, or the mismatches left). A percentage always sits beside its counts.
