# Fan header availability

Method and path: PUT /api/v1/fans/headers/{h}/availability.

Area: Fans.

## Request

**PUT /api/v1/fans/headers/{h}/availability**

Chooses whether the fan service shows one fan header; null leaves the choice to the board's own table.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"available":false}' "http://127.0.0.1:9430/api/v1/fans/headers/<h>/availability"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/fans/headers/<h>/availability", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"available":false}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `PutFanHeaderAvailability`.

| Field | Type | What it holds |
| --- | --- | --- |
| `available` | `boolean or null` | False hides the header, true shows it anyway, null leaves the board's own table in charge. Optional. |

**Answer body**: `JSON object`.

The fields of `FansStatus`, the fan status after the change. When the status cannot be read again, the answer carries the saved status with this header changed; when no status is saved, the answer carries only `ok`.

| Field | Type | What it holds |
| --- | --- | --- |
| `chip` | `string or null` | The fan chip once it is detected. |
| `hardware_watchdog` | `boolean or null` | Whether a hardware watchdog is reachable; null while that is still being settled. |
| `headers` | `array of JSON objects` | One entry per fan header, with its mode and state. |
| `temps` | `array of JSON objects` | The temperature readings the service reports. |
| `active_profile` | `string` | The profile in force. |
| `profiles` | `array of strings` | The profiles the service knows. |
| `floor_percent` | `number` | The lowest duty the service allows. |
| `hot` | `array of JSON objects` | The hot line in force for each source kind. |

| Path | Type | What it holds |
| --- | --- | --- |
| `h` | `string` | The id of a fan header, such as cpu_fan. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/fans` | none | FansStatus |
| PATCH `/api/v1/fans/headers/{h}` | FanMode | JSON object |
| PUT `/api/v1/fans/headers/{h}/availability` | PutFanHeaderAvailability | JSON object |
| PUT `/api/v1/fans/hot` | HotLimits | FansStatus |
| GET `/api/v1/fans/profile` | none | FanProfileName |
| PATCH `/api/v1/fans/profile` | FanProfileName | FanProfileName |
| PUT `/api/v1/fans/profiles/{name}` | FanProfile | FanProfile |
| DELETE `/api/v1/fans/profiles/{name}` | none | JSON object |

## Example

**PUT**

Chooses whether the fan service shows one fan header; null leaves the choice to the board's own table.

```json
{
  "available": false
}
```
