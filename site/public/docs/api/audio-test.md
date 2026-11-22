# Audio test

Method and path: GET / PUT / DELETE /api/v1/audio/test.

Area: Audio.

## Request

**GET /api/v1/audio/test**

Reads and renews an audio test lease with the id the PUT returned; an unknown or expired lease answers 410.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/audio/test?lease=<lease_id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/audio/test?lease=<lease_id>", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PUT /api/v1/audio/test**

Tests an audio endpoint and issues a lease. Set listen true for an input so the microphone opens, or leave it false to test an output; every PUT issues a lease.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"listen":false}' "http://127.0.0.1:9430/api/v1/audio/test"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/audio/test", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"listen":false}),
});
const data = await response.json();
```

**DELETE /api/v1/audio/test**

Ends the audio test lease with the id the PUT returned.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/audio/test?lease=<lease_id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/audio/test?lease=<lease_id>", {
  method: "DELETE",
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

**Answer body**: `AudioTestState`.

| Field | Type | What it holds |
| --- | --- | --- |
| `status` | `AudioTestStatus` | The state of the audio test. |
| `source` | `ResolvedAudioSource` | The audio source the test is using. |
| `sample_rate` | `number or null` | The sample rate the capture reports, in hertz. |
| `channels` | `number or null` | The channel count the capture reports. |
| `bits_per_sample` | `number or null` | The bit depth the capture device reports. |
| `format_tag` | `number or null` | The audio format the capture device reports. |
| `peak` | `number or null` | The peak level the capture meter reports, from 0 to 1, before the endpoint's own volume. |
| `peak_db` | `number or null` | The peak level in dBFS, floored at the silence floor. |
| `bands` | `array of number` | The bands the effects read, from 0 to 1. Empty when nothing is captured. |
| `packets` | `integer` | How many packets arrived in the last interval. |
| `silent_packets` | `integer` | How many of those packets were silent. |

| Query | Type | What it holds |
| --- | --- | --- |
| `lease` | `string` | The lease_id returned by the audio test PUT. Paste that value into the URL. |

**PUT**

**Request body**: `AudioTestRequest`.

| Field | Type | What it holds |
| --- | --- | --- |
| `listen` | `boolean` | Listen to the picked input. An output is tested without it; an input needs it. Leaving it out means false. Optional. |

**Answer body**: `AudioTestLease`.

| Field | Type | What it holds |
| --- | --- | --- |
| `lease_id` | `string` | The id returned when the test lease started. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `JSON object`.

The fields the daemon sends: the lease was released.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the lease was released. |

| Query | Type | What it holds |
| --- | --- | --- |
| `lease` | `string` | The lease_id returned by the audio test PUT. Paste that value into the URL. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| PUT `/api/v1/audio/source` | AudioSource | ResolvedAudioSource |
| GET `/api/v1/audio/sources` | none | AudioSources |
| GET `/api/v1/audio/test` | none | AudioTestState |
| PUT `/api/v1/audio/test` | AudioTestRequest | AudioTestLease |
| DELETE `/api/v1/audio/test` | none | JSON object |

## Example

**GET**

Reads and renews an audio test lease with the id the PUT returned; an unknown or expired lease answers 410.

This route takes no request body.

**PUT**

Tests an audio endpoint and issues a lease. Set listen true for an input so the microphone opens, or leave it false to test an output; every PUT issues a lease.

```json
{
  "listen": false
}
```

**DELETE**

Ends the audio test lease with the id the PUT returned.

This route takes no request body.
