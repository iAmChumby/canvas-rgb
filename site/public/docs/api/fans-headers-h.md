# Fan header

Method and path: PATCH /api/v1/fans/headers/{h}.

Area: Fans.

## Request

**PATCH /api/v1/fans/headers/{h}**

Changes the mode and settings of one fan header. A percent below the configured floor answers 422: the floor is 20 percent, or 30 for the CPU and GPU headers, and a higher value can be configured. `fixed` needs `percent`, `source` needs its fields, and `curve` needs 2 to 8 `points` sorted by temperature.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"mode":"bios"}' "http://127.0.0.1:9430/api/v1/fans/headers/<h>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/fans/headers/<h>", {
  method: "PATCH",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"mode":"bios"}),
});
const data = await response.json();
```

## Types

**PATCH**

**Request body**: `FanMode`.

One of: "bios", "fixed", "curve".

| Field | Type | What it holds |
| --- | --- | --- |
| `mode` | `"bios" or "fixed" or "curve"` | The mode the header runs in: `bios`, `fixed` or `curve`. |
| `percent` | `number` | The duty the header runs at, from 0 to 100. Required for `fixed` and at or above the header's floor. Optional. |
| `source` | `TempSource` | The temperature source a `curve` reads: `cpu`, `gpu` or a named board sensor. Required for `curve`. Optional. |
| `points` | `array of CurvePoint` | The points of a `curve`, 2 to 8, sorted by temperature. Optional. |

**Answer body**: `JSON object`.

The fields the daemon sends: the header whose mode changed and the mode that was set.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the mode was accepted. |
| `header` | `string` | The header whose mode changed. |
| `mode` | `JSON object` | The mode that was set. |

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

**PATCH**

Changes the mode and settings of one fan header. A percent below the configured floor answers 422: the floor is 20 percent, or 30 for the CPU and GPU headers, and a higher value can be configured. `fixed` needs `percent`, `source` needs its fields, and `curve` needs 2 to 8 `points` sorted by temperature.

```json
{
  "mode": "bios"
}
```
