# Flash lights

Method and path: POST /api/v1/lights/flash.

Area: Devices and lights.

## Request

**POST /api/v1/lights/flash**

Flashes the named lights white and leaves every other LED dark.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"lights":["openrgb:2a3b4c5d#z0"],"stop":false}' "http://127.0.0.1:9430/api/v1/lights/flash"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/lights/flash", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"lights":["openrgb:2a3b4c5d#z0"],"stop":false}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `FlashRequest`.

| Field | Type | What it holds |
| --- | --- | --- |
| `lights` | `array of string` | The ids of the lights to flash. Optional. |
| `stop` | `boolean` | True stops the flash. Optional. |

**Answer body**: `JSON object`.

The fields the daemon sends: the flash was started or stopped.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the flash was started or stopped. |

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

Flashes the named lights white and leaves every other LED dark.

```json
{
  "lights": [
    "openrgb:2a3b4c5d#z0"
  ],
  "stop": false
}
```
