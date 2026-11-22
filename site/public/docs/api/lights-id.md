# Light

Method and path: PATCH / DELETE /api/v1/lights/{id}.

Area: Devices and lights.

## Request

**PATCH /api/v1/lights/{id}**

Changes one light's name, arrangement or other saved settings. A name that is empty or longer than the group name limit answers 422; clear a name with null instead.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"name":"Front fans","arrangement":"ring"}' "http://127.0.0.1:9430/api/v1/lights/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/lights/<id>", {
  method: "PATCH",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"name":"Front fans","arrangement":"ring"}),
});
const data = await response.json();
```

**DELETE /api/v1/lights/{id}**

Removes a leftover light from an older configuration; a light on a device the daemon drives answers 409.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/lights/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/lights/<id>", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**PATCH**

**Request body**: `LightPatch`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string or null` | The name to give the light. Optional. |
| `arrangement` | `Arrangement` | How the LEDs are laid out, such as `ring` or `strip`. Optional. |
| `reversed` | `boolean` | True walks the LEDs the other way. Optional. |
| `kind` | `ChainPartKind` | What the part is. Setting it sets `arrangement` from it; setting `arrangement` alone leaves `kind` as it was. Optional. |
| `first_led` | `number` | Which LED of the run is the first one. 0 starts the run at the first LED. It must be below the light's LED count. Optional. |

**Answer body**: `LightView`.

| Field | Type | What it holds |
| --- | --- | --- |
| `light` | `Light` | The light itself, with its id, device, zone and run of LEDs. |
| `led_count` | `number` | The LEDs the light has now: its run clamped to the zone, or the zone's count. |
| `connected` | `boolean` | True when the light's device is attached right now. |
| `label` | `string` | The name the person gave the light, or the device and zone name. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `LightDeleted`.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the light was removed. |
| `id` | `string` | The id of the light that was removed. |
| `emptied_groups` | `array of EmptiedGroup` | The groups the light was the last member of; each was dropped with it. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the light the route names. |

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

Changes one light's name, arrangement or other saved settings. A name that is empty or longer than the group name limit answers 422; clear a name with null instead.

```json
{
  "name": "Front fans",
  "arrangement": "ring"
}
```

**DELETE**

Removes a leftover light from an older configuration; a light on a device the daemon drives answers 409.

This route takes no request body.
