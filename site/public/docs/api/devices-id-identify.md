# Identify device

Method and path: POST /api/v1/devices/{id}/identify.

Area: Devices and lights.

## Request

**POST /api/v1/devices/{id}/identify**

Flashes one target LED so the device holding it can be found in the room. An unknown zone or an LED past the end of the zone answers 422.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"zone":0,"led":0,"stop":false}' "http://127.0.0.1:9430/api/v1/devices/<id>/identify"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/devices/<id>/identify", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"zone":0,"led":0,"stop":false}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `IdentifyRequest`.

| Field | Type | What it holds |
| --- | --- | --- |
| `zone` | `number` | The zone index to flash, starting at 0. |
| `led` | `number` | The index of the LED to flash. |
| `count` | `number` | How many LEDs to flash from `led`, such as a whole fan. Leave it out for one. Optional. |
| `stop` | `boolean` | True stops the flash. Optional. |

**Answer body**: `JSON object`.

The fields the daemon sends: the flash was started or stopped.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the flash was started or stopped. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the device to flash. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/devices` | none | DevicesResponse |
| POST `/api/v1/devices/{id}/identify` | IdentifyRequest | JSON object |
| PUT `/api/v1/devices/{id}/white-balance` | WhiteBalance | WhiteBalances |
| DELETE `/api/v1/devices/{id}/white-balance` | none | WhiteBalances |
| PATCH `/api/v1/devices/{id}/zones/{zone}` | PatchZone | JSON object |
| GET `/api/v1/lights` | none | LightsResponse |
| PATCH `/api/v1/lights/{id}` | LightPatch | LightView |
| DELETE `/api/v1/lights/{id}` | none | LightDeleted |
| POST `/api/v1/lights/flash` | FlashRequest | JSON object |
| GET `/api/v1/white-balance` | none | WhiteBalances |

## Example

**POST**

Flashes one target LED so the device holding it can be found in the room. An unknown zone or an LED past the end of the zone answers 422.

```json
{
  "zone": 0,
  "led": 0,
  "stop": false
}
```
