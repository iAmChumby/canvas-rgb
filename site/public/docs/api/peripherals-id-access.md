# Peripheral access

Method and path: PUT /api/v1/peripherals/{id}/access.

Area: Displays and peripherals.

## Request

**PUT /api/v1/peripherals/{id}/access**

Turns reads and configuration writes on or off for one peripheral. `writes` without `reads` answers 422, because a write patches what the device last reported.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"reads":true,"writes":true}' "http://127.0.0.1:9430/api/v1/peripherals/<id>/access"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/peripherals/<id>/access", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"reads":true,"writes":true}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `PeripheralAccess`.

| Field | Type | What it holds |
| --- | --- | --- |
| `reads` | `boolean` | True opens the device and reads its settings. A read may send the device a request that changes no setting. |
| `writes` | `boolean` | True sends config writes. It needs reads, because a write patches what the device last reported; `writes` without `reads` answers 422. |

**Answer body**: `PeripheralAccess`.

| Field | Type | What it holds |
| --- | --- | --- |
| `reads` | `boolean` | True opens the device and reads its settings. A read may send the device a request that changes no setting. |
| `writes` | `boolean` | True sends config writes. It needs reads, because a write patches what the device last reported; `writes` without `reads` answers 422. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the peripheral to open or close. |

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

Turns reads and configuration writes on or off for one peripheral. `writes` without `reads` answers 422, because a write patches what the device last reported.

```json
{
  "reads": true,
  "writes": true
}
```
