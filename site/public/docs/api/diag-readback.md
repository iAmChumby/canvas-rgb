# Readback

Method and path: GET /api/v1/diag/readback.

Area: Diagnostics.

## Request

**GET /api/v1/diag/readback**

Reads the colours the hardware reports back from the devices.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/diag/readback"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/diag/readback", {
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

**Answer body**: `ReadbackResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `devices` | `array of ReadbackDevice` | The devices with the colours read back from their LEDs. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| POST `/api/v1/diag/fail-sensor` | FailSensorRequest | JSON object |
| GET `/api/v1/diag/readback` | none | ReadbackResponse |

## Example

**GET**

Reads the colours the hardware reports back from the devices.

This route takes no request body.
