# Peripheral

Method and path: GET / PATCH /api/v1/peripherals/{id}.

Area: Displays and peripherals.

## Request

**GET /api/v1/peripherals/{id}**

Reports one peripheral's schema and last-read values; ?fresh=true reads the device now.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/peripherals/<id>?fresh=true"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/peripherals/<id>?fresh=true", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PATCH /api/v1/peripherals/{id}**

Changes one peripheral's settings.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"brightness":{"type":"number","value":30}}' "http://127.0.0.1:9430/api/v1/peripherals/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/peripherals/<id>", {
  method: "PATCH",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"brightness":{"type":"number","value":30}}),
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `PeripheralDetail`.

| Field | Type | What it holds |
| --- | --- | --- |
| `info` | `PeripheralInfo` | The peripheral itself. |
| `schema` | `PeripheralSchema` | The peripheral's sections and properties. |
| `values` | `an object keyed by text, with PropValue values` | The values read from the device, by property key. A key the device did not report this time is absent; all of them are absent while reads are off. |
| `access` | `PeripheralAccess` | What the daemon may do with this device. |
| `problem` | `string or null` | Why the values are missing, when they are: reads are off, the device did not answer or the vendor app holds it. It is null after a good read. |
| `read_at_ms` | `integer` | When the values were last read from the device, in Unix milliseconds. The daemon answers from kept values, so the editor shows how old they are. Optional. |

| Query | Type | What it holds |
| --- | --- | --- |
| `fresh` | `boolean` | True reads the device now. Without it the values are the ones the daemon kept from the last read. |

**PATCH**

**Request body**: `Props`.

A map of property id to PropValue.

**Answer body**: `Props`.

A map of property id to PropValue.

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the peripheral the route names. |

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

Reports one peripheral's schema and last-read values; ?fresh=true reads the device now.

This route takes no request body.

**PATCH**

Changes one peripheral's settings.

```json
{
  "brightness": {
    "type": "number",
    "value": 30
  }
}
```
