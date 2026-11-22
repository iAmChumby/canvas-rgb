# Import effect

Method and path: POST /api/v1/effects/import.

Area: Effects.

## Request

**POST /api/v1/effects/import**

Validates an effect file and installs it in the library.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"file_name":"new-effect.html","html":"<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>","replace":false}' "http://127.0.0.1:9430/api/v1/effects/import"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/import", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"file_name":"new-effect.html","html":"<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>","replace":false}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `ImportEffect`.

| Field | Type | What it holds |
| --- | --- | --- |
| `file_name` | `string` | The file name to save it as: letters, digits, space, `-`, `_` and `.`, ending in `.html`. No folders. The name is refused when it holds `..`, when it starts or ends with a space or a dot, when the part before the first dot is a name Windows reserves, or when it would carry a built-in effect's id. |
| `html` | `string` | The file's contents, at most 1 MiB of bytes. |
| `replace` | `boolean` | True overwrites a user effect of the same name; without it a file of that name is refused with a conflict. |

**Answer body**: `ImportEffectResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `effect` | `EffectSummary` | The effect that was imported. |
| `replaced` | `boolean` | True when an existing file was overwritten. |

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

Validates an effect file and installs it in the library.

```json
{
  "file_name": "new-effect.html",
  "html": "<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>",
  "replace": false
}
```
