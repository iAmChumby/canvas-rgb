# Display

Method and path: PUT / DELETE /api/v1/displays/{id}.

Area: Displays and peripherals.

## Request

**PUT /api/v1/displays/{id}**

Changes the settings of one display.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"device":"display:2b1c:3d4e","source":{"kind":"api"},"unit":"celsius","decimals":0,"blink_above":80}' "http://127.0.0.1:9430/api/v1/displays/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/displays/<id>", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"device":"display:2b1c:3d4e","source":{"kind":"api"},"unit":"celsius","decimals":0,"blink_above":80}),
});
const data = await response.json();
```

**DELETE /api/v1/displays/{id}**

Removes one display's saved config and closes the device.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/displays/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/displays/<id>", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `DisplayConfig`.

| Field | Type | What it holds |
| --- | --- | --- |
| `device` | `DeviceId` | The display the config belongs to. |
| `source` | `DisplaySource` | Where the display's number comes from. |
| `unit` | `DisplayUnit` | The unit the number is shown in: `celsius` or `fahrenheit`. |
| `decimals` | `number` | How many digits follow the decimal point. |
| `blink_above` | `number or null` | The value at or above which the display blinks; null leaves it steady. Optional. |

**Answer body**: `DisplayConfig`.

| Field | Type | What it holds |
| --- | --- | --- |
| `device` | `DeviceId` | The display the config belongs to. |
| `source` | `DisplaySource` | Where the display's number comes from. |
| `unit` | `DisplayUnit` | The unit the number is shown in: `celsius` or `fahrenheit`. |
| `decimals` | `number` | How many digits follow the decimal point. |
| `blink_above` | `number or null` | The value at or above which the display blinks; null leaves it steady. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `JSON object`.

The fields the daemon sends: the display whose config was removed.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the saved config was removed. |
| `device_id` | `string` | The display whose config was removed. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the display to change. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/displays` | none | array of DisplayStatus |
| PUT `/api/v1/displays/{id}` | DisplayConfig | DisplayConfig |
| DELETE `/api/v1/displays/{id}` | none | JSON object |
| PUT `/api/v1/displays/{id}/value` | SetDisplayValue | SetDisplayValue |
| GET `/api/v1/peripherals` | none | PeripheralsResponse |
| GET `/api/v1/peripherals/{id}` | none | PeripheralDetail |
| PATCH `/api/v1/peripherals/{id}` | Props | Props |
| PUT `/api/v1/peripherals/{id}/access` | PeripheralAccess | PeripheralAccess |
| GET `/api/v1/peripherals/{id}/read` | none | ReadProgress |

## Example

**PUT**

Changes the settings of one display.

```json
{
  "device": "display:2b1c:3d4e",
  "source": {
    "kind": "api"
  },
  "unit": "celsius",
  "decimals": 0,
  "blink_above": 80
}
```

**DELETE**

Removes one display's saved config and closes the device.

This route takes no request body.
