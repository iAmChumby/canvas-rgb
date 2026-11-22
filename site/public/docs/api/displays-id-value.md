# Display value

Method and path: PUT /api/v1/displays/{id}/value.

Area: Displays and peripherals.

## Request

**PUT /api/v1/displays/{id}/value**

Sets the number shown on a display whose value comes from this API. A display whose saved source is not the API answers 422.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"value":23.5}' "http://127.0.0.1:9430/api/v1/displays/<id>/value"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/displays/<id>/value", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"value":23.5}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `SetDisplayValue`.

| Field | Type | What it holds |
| --- | --- | --- |
| `value` | `number or null` | The number to show on the display. Optional. |

**Answer body**: `SetDisplayValue`.

| Field | Type | What it holds |
| --- | --- | --- |
| `value` | `number or null` | The number to show on the display. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the display to write to. |

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

Sets the number shown on a display whose value comes from this API. A display whose saved source is not the API answers 422.

```json
{
  "value": 23.5
}
```
