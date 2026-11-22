# Fail sensor

Method and path: POST /api/v1/diag/fail-sensor.

Area: Diagnostics.

## Request

**POST /api/v1/diag/fail-sensor**

Makes a temperature source report a failure, for a safety test. The source is `cpu`, `gpu` or `board:<name>`; any other value answers 422.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"source":"cpu","failed":true}' "http://127.0.0.1:9430/api/v1/diag/fail-sensor"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/diag/fail-sensor", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"source":"cpu","failed":true}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `FailSensorRequest`.

The temperature source to mark as failed and whether to mark it.

| Field | Type | What it holds |
| --- | --- | --- |
| `source` | `string` | The temperature source to change: `cpu`, `gpu` or `board:<name>`. |
| `failed` | `boolean` | True marks the source as failed. Optional. |

**Answer body**: `JSON object`.

The fields the daemon sends: the temperature source that was told to fail and whether it now reports a failure.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the failure flag was applied. |
| `source` | `JSON object` | The temperature source told to fail. |
| `failed` | `boolean` | True when the source now reports a failure. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| POST `/api/v1/diag/fail-sensor` | FailSensorRequest | JSON object |
| GET `/api/v1/diag/readback` | none | ReadbackResponse |

## Example

**POST**

Makes a temperature source report a failure, for a safety test. The source is `cpu`, `gpu` or `board:<name>`; any other value answers 422.

```json
{
  "source": "cpu",
  "failed": true
}
```
