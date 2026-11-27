# Settings

Method and path: GET / PUT /api/v1/settings.

Area: Settings and update.

## Request

**GET /api/v1/settings**

Returns the current settings.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/settings"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/settings", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PUT /api/v1/settings**

Replaces the settings.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"fps":30,"black_on_lock":false,"on_quit":"black","boot_color":"#1b2c3d","brightness":80,"power":"running","warmth":6500}' "http://127.0.0.1:9430/api/v1/settings"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/settings", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"fps":30,"black_on_lock":false,"on_quit":"black","boot_color":"#1b2c3d","brightness":80,"power":"running","warmth":6500}),
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `Settings`.

| Field | Type | What it holds |
| --- | --- | --- |
| `fps` | `number` | The frame rate target, from 10 to 60. |
| `black_on_lock` | `boolean` | True turns the lights off while the screen is locked. It is off by default. |
| `on_quit` | `OnQuit` | The lighting to leave on at exit: `black` or `leave_last_frame`. |
| `boot_color` | `Rgb8 or null` | The colour shown before the daemon connects, as a hex string such as #1b2c3d; null means black. |
| `brightness` | `number` | The brightness of the output, from 0 to 100. |
| `power` | `Power` | Whether the render loop runs or pauses. |
| `warmth` | `number` | The colour temperature in kelvin, from 1800 to 6500; 6500 leaves the picture unchanged. |

**PUT**

**Request body**: `Settings`.

| Field | Type | What it holds |
| --- | --- | --- |
| `fps` | `number` | The frame rate target, from 10 to 60. |
| `black_on_lock` | `boolean` | True turns the lights off while the screen is locked. It is off by default. |
| `on_quit` | `OnQuit` | The lighting to leave on at exit: `black` or `leave_last_frame`. |
| `boot_color` | `Rgb8 or null` | The colour shown before the daemon connects, as a hex string such as #1b2c3d; null means black. Optional. |
| `brightness` | `number` | The brightness of the output, from 0 to 100. |
| `power` | `Power` | Whether the render loop runs or pauses. |
| `warmth` | `number` | The colour temperature in kelvin, from 1800 to 6500; 6500 leaves the picture unchanged. Optional. |

**Answer body**: `Settings`.

| Field | Type | What it holds |
| --- | --- | --- |
| `fps` | `number` | The frame rate target, from 10 to 60. |
| `black_on_lock` | `boolean` | True turns the lights off while the screen is locked. It is off by default. |
| `on_quit` | `OnQuit` | The lighting to leave on at exit: `black` or `leave_last_frame`. |
| `boot_color` | `Rgb8 or null` | The colour shown before the daemon connects, as a hex string such as #1b2c3d; null means black. |
| `brightness` | `number` | The brightness of the output, from 0 to 100. |
| `power` | `Power` | Whether the render loop runs or pauses. |
| `warmth` | `number` | The colour temperature in kelvin, from 1800 to 6500; 6500 leaves the picture unchanged. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/config/export` | none | ExportBundle |
| POST `/api/v1/config/import` | ExportBundle | JSON object |
| POST `/api/v1/first-run/dismiss-zone-panel` | none | JSON object |
| POST `/api/v1/onboarding/complete` | none | JSON object |
| GET `/api/v1/settings` | none | Settings |
| PUT `/api/v1/settings` | Settings | Settings |
| POST `/api/v1/update/install` | InstallUpdateRequest | InstallUpdateResponse |

## Example

**GET**

Returns the current settings.

This route takes no request body.

**PUT**

Replaces the settings.

```json
{
  "fps": 30,
  "black_on_lock": false,
  "on_quit": "black",
  "boot_color": "#1b2c3d",
  "brightness": 80,
  "power": "running",
  "warmth": 6500
}
```
