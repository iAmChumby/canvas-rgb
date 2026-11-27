# Status

Method and path: GET /api/v1/status.

Area: Status and output.

## Request

**GET /api/v1/status**

Reports the daemon's version, power state, brightness, frame rate, connected devices, active scene and the links to OpenRGB and to the fan and update service.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET "http://127.0.0.1:9430/api/v1/status"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/status", {
  method: "GET",
  headers: {
  },
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `Status`.

| Field | Type | What it holds |
| --- | --- | --- |
| `version` | `string` | The version of the daemon. |
| `power` | `Power` | Whether the render loop runs or pauses. |
| `brightness` | `number` | The brightness of the output, from 0 to 100. |
| `fps_target` | `number` | The frame rate the daemon aims for. |
| `fps_measured` | `number` | The frame rate measured over the last second. |
| `frame_time_ms` | `number` | The time the last frame took, in milliseconds. |
| `frames_rendered` | `number` | How many frames the daemon has rendered since it started. The count stops growing while the loop is parked or paused. |
| `parked` | `boolean` | True when the render loop has nothing to draw for: no device the daemon drives wants frames and no preview client is watching. |
| `devices_connected` | `number` | How many devices are connected. |
| `devices_total` | `number` | How many devices the daemon knows about, connected or not. The editor draws one card per device while the list is on its way, so it needs the total and the connected count. |
| `preview_subscribers` | `number` | How many preview clients are attached. |
| `active_scene` | `SceneRef or null` | The active scene, when one is set. Optional. |
| `active_effects` | `array of string` | The ids of the effects the active scene's groups play, in group order, without repeats. |
| `openrgb` | `LinkState` | The state of the OpenRGB connection. |
| `service` | `LinkState` | The state of the link to the fan and update service. |
| `service_down_ms` | `number or null` | How long the link to the fan and update service has been down, in milliseconds, when it is down. |
| `conflicts` | `array of Conflict` | The conflicts that need attention: SignalRGB is running and will fight canvas-rgb for the devices. The list is empty when SignalRGB is not running. |
| `show_zone_panel` | `boolean` | True shows the editor's dismissible panel for sizing the ARGB headers. |
| `onboarding_complete` | `boolean` | False until the person finishes the first-run onboarding. While it is false the tray opens the window by itself onto the onboarding. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/about` | none | About |
| GET `/api/v1/about/report` | none | AboutReport |
| GET `/api/v1/output` | none | OutputState |
| PATCH `/api/v1/output` | PatchOutput | OutputState |
| POST `/api/v1/quit` | none | JSON object |
| GET `/api/v1/status` | none | Status |

## Example

**GET**

Reports the daemon's version, power state, brightness, frame rate, connected devices, active scene and the links to OpenRGB and to the fan and update service.

This route takes no request body.
