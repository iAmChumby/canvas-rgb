# Device white balance

Method and path: PUT / DELETE /api/v1/devices/{id}/white-balance.

Area: Devices and lights.

## Request

**PUT /api/v1/devices/{id}/white-balance**

Sets the red, green and blue gains of one device. Every gain must be a number from 0 to 1; any gain outside that range answers 422.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"red":1,"green":1,"blue":1}' "http://127.0.0.1:9430/api/v1/devices/<id>/white-balance"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/devices/<id>/white-balance", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"red":1,"green":1,"blue":1}),
});
const data = await response.json();
```

**DELETE /api/v1/devices/{id}/white-balance**

Resets one device's gains to neutral.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/devices/<id>/white-balance"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/devices/<id>/white-balance", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `WhiteBalance`.

| Field | Type | What it holds |
| --- | --- | --- |
| `red` | `number` | The red gain, as a multiplier. |
| `green` | `number` | The green gain, as a multiplier. |
| `blue` | `number` | The blue gain, as a multiplier. |

**Answer body**: `WhiteBalances`.

| Field | Type | What it holds |
| --- | --- | --- |
| `devices` | `an object keyed by text, with WhiteBalance values` | The colour gains of each device, by device id. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `WhiteBalances`.

| Field | Type | What it holds |
| --- | --- | --- |
| `devices` | `an object keyed by text, with WhiteBalance values` | The colour gains of each device, by device id. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the device whose gains are set. |

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

**PUT**

Sets the red, green and blue gains of one device. Every gain must be a number from 0 to 1; any gain outside that range answers 422.

```json
{
  "red": 1,
  "green": 1,
  "blue": 1
}
```

**DELETE**

Resets one device's gains to neutral.

This route takes no request body.
