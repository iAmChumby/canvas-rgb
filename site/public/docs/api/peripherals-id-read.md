# Peripheral read

Method and path: GET /api/v1/peripherals/{id}/read.

Area: Displays and peripherals.

## Request

**GET /api/v1/peripherals/{id}/read**

Reports the progress of the read running on one peripheral.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/peripherals/<id>/read"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/peripherals/<id>/read", {
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

**Answer body**: `ReadProgress`.

| Field | Type | What it holds |
| --- | --- | --- |
| `stage` | `ReadStage` | The stage of the read. |
| `done` | `number` | The reports read so far while the stage is reading; 0 in the other stages. |
| `total` | `number` | The reports the read will make; 0 when the adapter does not know a count. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the peripheral whose read is watched. |

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

Reports the progress of the read running on one peripheral.

This route takes no request body.
