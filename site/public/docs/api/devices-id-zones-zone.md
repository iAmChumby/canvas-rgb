# Device zone

Method and path: PATCH /api/v1/devices/{id}/zones/{zone}.

Area: Devices and lights.

## Request

**PATCH /api/v1/devices/{id}/zones/{zone}**

Changes the LED count of one zone on a device. An LED count outside the zone's range or a zone that cannot be resized answers 422.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"led_count":512}' "http://127.0.0.1:9430/api/v1/devices/<id>/zones/<zone>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/devices/<id>/zones/<zone>", {
  method: "PATCH",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"led_count":512}),
});
const data = await response.json();
```

## Types

**PATCH**

**Request body**: `PatchZone`.

| Field | Type | What it holds |
| --- | --- | --- |
| `led_count` | `number` | How many LEDs the zone holds. |

**Answer body**: `JSON object`.

The fields the daemon sends: the zone was resized, with the device and the zone it belongs to.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the zone was resized. |
| `device_id` | `string` | The device the zone belongs to. |
| `zone` | `number` | The zone's index. |
| `led_count` | `number` | The zone's new LED count. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the device whose zone is resized. |
| `zone` | `number` | The index of the zone, starting at 0. |

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

**PATCH**

Changes the LED count of one zone on a device. An LED count outside the zone's range or a zone that cannot be resized answers 422.

```json
{
  "led_count": 512
}
```
