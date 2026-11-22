# Audio source

Method and path: PUT /api/v1/audio/source.

Area: Audio.

## Request

**PUT /api/v1/audio/source**

Chooses the audio source.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"kind":"follow_default"}' "http://127.0.0.1:9430/api/v1/audio/source"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/audio/source", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"kind":"follow_default"}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `AudioSource`.

One of: "follow_default", "endpoint".

| Field | Type | What it holds |
| --- | --- | --- |
| `kind` | `"follow_default" or "endpoint"` | The kind of source to use: `follow_default` follows the system pick, `endpoint` names a device. |
| `id` | `string` | The id of the endpoint to use. Optional. |
| `flow` | `AudioFlow` | Whether the endpoint is an input or an output. Optional. |
| `name_hint` | `string` | A label to show for the chosen endpoint. Optional. |

**Answer body**: `ResolvedAudioSource`.

| Field | Type | What it holds |
| --- | --- | --- |
| `requested` | `AudioSource` | The audio source that was picked. |
| `endpoint_id` | `string or null` | The endpoint the capture opened or will open; null when there is no endpoint at all. |
| `name` | `string or null` | The name of the endpoint. |
| `flow` | `AudioFlow or null` | Whether the endpoint is an input or an output. |
| `fallback` | `AudioFallback or null` | Why the capture is not on the picked endpoint. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| PUT `/api/v1/audio/source` | AudioSource | ResolvedAudioSource |
| GET `/api/v1/audio/sources` | none | AudioSources |
| GET `/api/v1/audio/test` | none | AudioTestState |
| PUT `/api/v1/audio/test` | AudioTestRequest | AudioTestLease |
| DELETE `/api/v1/audio/test` | none | JSON object |

## Example

**PUT**

Chooses the audio source.

```json
{
  "kind": "follow_default"
}
```
