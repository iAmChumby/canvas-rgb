# Fan hot temperatures

Method and path: PUT /api/v1/fans/hot.

Area: Fans.

## Request

**PUT /api/v1/fans/hot**

Sets the temperatures at which fans are driven at full duty.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"cpu_c":75,"gpu_c":null,"board_c":null}' "http://127.0.0.1:9430/api/v1/fans/hot"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/fans/hot", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"cpu_c":75,"gpu_c":null,"board_c":null}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `HotLimits`.

| Field | Type | What it holds |
| --- | --- | --- |
| `cpu_c` | `number or null` | The CPU temperature for the hot limit, in degrees Celsius. Optional. |
| `gpu_c` | `number or null` | The GPU temperature for the hot limit, in degrees Celsius. Optional. |
| `board_c` | `number or null` | The board temperature for the hot limit, in degrees Celsius. Optional. |

**Answer body**: `FansStatus`.

| Field | Type | What it holds |
| --- | --- | --- |
| `chip` | `string or null` | The name of the fan chip, once the fan service has detected it. |
| `hardware_watchdog` | `boolean or null` | Whether the chip has a hardware fan watchdog reachable via PawnIO; null while that is still being settled. |
| `headers` | `array of FanHeaderStatus` | One entry per fan header, with its mode and state. |
| `temps` | `array of TempReading` | The temperature readings the fan service reports. |
| `active_profile` | `string` | The fan profile in force. |
| `profiles` | `array of string` | The names of the profiles the fan service knows. |
| `floor_percent` | `number` | The lowest duty the fan service allows, in percent. |
| `hot` | `array of HotLimitStatus` | The hot line in force per kind of source, for the curve chart. Old payloads without it parse as empty. |

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

Sets the temperatures at which fans are driven at full duty.

```json
{
  "cpu_c": 75,
  "gpu_c": null,
  "board_c": null
}
```
