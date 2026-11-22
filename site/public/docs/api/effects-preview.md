# Effect preview

Method and path: POST /api/v1/effects/preview.

Area: Effects.

## Request

**POST /api/v1/effects/preview**

Renders a draft HTML effect with the given property values.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"html":"<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>","props":{}}' "http://127.0.0.1:9430/api/v1/effects/preview"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/preview", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"html":"<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>","props":{}}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `EffectPreviewRequest`.

| Field | Type | What it holds |
| --- | --- | --- |
| `html` | `string` | The draft HTML, at most 1 MiB of bytes. |
| `props` | `an object keyed by text, with PropValue values` | The property values the preview is rendered with. |

**Answer body**: `EffectPreviewResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `schema` | `PropertySchema` | The draft's property schema. |
| `source_hash` | `string` | The hash of the exact HTML the preview was rendered from. |
| `errors` | `array of EffectError` | What the runtime reported while the draft was parsed and drawn. A preview with errors still carries the loop, so the last good frame stays drawn. |
| `warnings` | `array of string` | The check's warnings: a picture that does not move, one that is one flat colour, and anything the file declared that the loader ignored. |
| `thumbnail` | `string` | The 64 by 40, 24-frame loop rendered with the editor's property values, as base64. Decoded it is the thumbnail format: three 16-bit little-endian counts (width, height, frame count) followed by the raw RGB frames. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/effects` | none | EffectsResponse |
| GET `/api/v1/effects/{id}` | none | EffectDetail |
| PUT `/api/v1/effects/{id}` | EffectDetail | JSON object |
| DELETE `/api/v1/effects/{id}` | none | DeleteEffectResponse |
| POST `/api/v1/effects/{id}/activate` | ActivateEffect | JSON object |
| PATCH `/api/v1/effects/{id}/defaults` | Props | JSON object |
| PUT `/api/v1/effects/{id}/favorite` | SetFavorite | JSON object |
| POST `/api/v1/effects/{id}/presets` | PresetCreate | PresetsResponse |
| PUT `/api/v1/effects/{id}/presets/{name}` | PresetReplace | PresetsResponse |
| DELETE `/api/v1/effects/{id}/presets/{name}` | none | PresetsResponse |
| GET `/api/v1/effects/{id}/source` | none | EffectSource |
| GET `/api/v1/effects/{id}/thumbnail` | none | Binary thumbnail |
| GET `/api/v1/effects/draft` | none | DraftStatus |
| PUT `/api/v1/effects/draft` | DraftRequest | DraftStatus |
| DELETE `/api/v1/effects/draft` | none | DraftStatus |
| POST `/api/v1/effects/import` | ImportEffect | ImportEffectResponse |
| POST `/api/v1/effects/preview` | EffectPreviewRequest | EffectPreviewResponse |

## Example

**POST**

Renders a draft HTML effect with the given property values.

```json
{
  "html": "<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>",
  "props": {}
}
```
