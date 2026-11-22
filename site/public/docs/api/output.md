# Output

Method and path: GET / PATCH /api/v1/output.

Area: Status and output.

## Request

**GET /api/v1/output**

Reports the current power state, brightness and warmth of the lighting output.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/output"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/output", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PATCH /api/v1/output**

Changes the power state, brightness or warmth of the lighting output.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"power":"running","brightness":80,"warmth":6500}' "http://127.0.0.1:9430/api/v1/output"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/output", {
  method: "PATCH",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"power":"running","brightness":80,"warmth":6500}),
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `OutputState`.

| Field | Type | What it holds |
| --- | --- | --- |
| `power` | `Power` | Whether the render loop runs or pauses. |
| `brightness` | `number` | The brightness of the output, from 0 to 100. |
| `warmth` | `number` | The warmth of the output in kelvin, from 1800 to 6500; 6500 leaves the picture unchanged. |

**PATCH**

**Request body**: `PatchOutput`.

| Field | Type | What it holds |
| --- | --- | --- |
| `power` | `Power or null` | Whether the render loop runs or pauses. A field left out is unchanged. Optional. |
| `brightness` | `number or null` | The brightness to set, from 0 to 100. A field left out is unchanged. Optional. |
| `warmth` | `number or null` | The warmth to set, in kelvin from 1800 to 6500. A value outside that range answers 422. A field left out is unchanged. Optional. |

**Answer body**: `OutputState`.

| Field | Type | What it holds |
| --- | --- | --- |
| `power` | `Power` | Whether the render loop runs or pauses. |
| `brightness` | `number` | The brightness of the output, from 0 to 100. |
| `warmth` | `number` | The warmth of the output in kelvin, from 1800 to 6500; 6500 leaves the picture unchanged. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/output` | none | OutputState |
| PATCH `/api/v1/output` | PatchOutput | OutputState |
| POST `/api/v1/quit` | none | JSON object |
| GET `/api/v1/status` | none | Status |

## Example

**GET**

Reports the current power state, brightness and warmth of the lighting output.

This route takes no request body.

**PATCH**

Changes the power state, brightness or warmth of the lighting output.

```json
{
  "power": "running",
  "brightness": 80,
  "warmth": 6500
}
```
