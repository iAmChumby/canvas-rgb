# Audio sources

Method and path: GET /api/v1/audio/sources.

Area: Audio.

## Request

**GET /api/v1/audio/sources**

Lists the audio endpoints available as sources and the resolved current choice.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/audio/sources"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/audio/sources", {
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

**Answer body**: `AudioSources`.

| Field | Type | What it holds |
| --- | --- | --- |
| `endpoints` | `array of AudioEndpoint` | The audio endpoints available as sources. |
| `current` | `ResolvedAudioSource` | The source the capture resolved the choice to. |

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

Lists the audio endpoints available as sources and the resolved current choice.

This route takes no request body.
