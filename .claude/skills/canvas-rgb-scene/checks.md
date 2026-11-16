# How to measure the checks

`SKILL.md` step 7 lists the checks. This file defines the terms of rows 2 and 8. Write the numbers
you computed in the report next to each row.

## Row 2: a built-in effect draws (`diag dump-frames`)

`canvas-rgb diag dump-frames --effect <id> --frames 30 --out <dir>` writes `frame_00000.rgb` to
`frame_00029.rgb` in `<dir>`. Each file holds one frame's pixels in row order, three bytes per
pixel (red, green, blue), so it is `width * height * 3` bytes. The defaults are `--width 320` and
`--height 200`. Read frames 0 to 29 and compute three things:

- Not black: at least one byte is above 0 in some frame. Report the count of non-zero bytes in
  frame 0 and in frame 29.
- Not one colour: some frame has at least two distinct red-green-blue triples. Report how many
  distinct triples frame 0 has.
- Changes: some byte differs between frame 0 and frame 29. Report how many bytes differ.

All three must hold for TRUE. Any one that fails is FALSE, with its number.

`dump-frames` refuses `html:` ids, so when no group in the plan plays a built-in effect, row 2 is
UNKNOWN: name the groups and say they all play HTML effects (row 1 covers those).

## Row 8: each group lit and moving (`diag readback`, scene playing only)

`canvas-rgb diag readback --json` prints `{"devices": [{"device_id": ..., "name": ..., "colors":
["#rrggbb", ...]}, ...]}`: one list of LED colours per device, in LED order. It does not name
zones. Run it three times, one second apart, and read each device that holds a member of the
scene:

- Not black: at least one colour in the device's list is not `#000000`.
- Two or more colours: the device's lists, taken together over the three samples, hold at least
  two distinct `#rrggbb` values.
- A change across samples: the list differs between at least two of the three samples.

A device that is not in the answer is UNKNOWN, and you name it. A device that holds members of
more than one group is judged as one device; say so.
