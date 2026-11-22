# White balance

Method and path: GET /api/v1/white-balance.

Area: Devices and lights.

## Request

**GET /api/v1/white-balance**

Lists each device's current colour gains.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/white-balance"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/white-balance", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `WhiteBalances`.

| Field | Type | What it holds |
| --- | --- | --- |
| `devices` | `an object keyed by text, with WhiteBalance values` | The colour gains of each device, by device id. |

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

**GET**

Lists each device's current colour gains.

This route takes no request body.
