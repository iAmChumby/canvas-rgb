# Peripherals

Method and path: GET /api/v1/peripherals.

Area: Displays and peripherals.

## Request

**GET /api/v1/peripherals**

Lists the peripherals the adapters see.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/peripherals"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/peripherals", {
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

**Answer body**: `PeripheralsResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `peripherals` | `array of PeripheralInfo` | The peripherals the adapters list. |

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

**GET**

Lists the peripherals the adapters see.

This route takes no request body.
