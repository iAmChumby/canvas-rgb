# Effect favourite

Method and path: PUT /api/v1/effects/{id}/favorite.

Area: Effects.

## Request

**PUT /api/v1/effects/{id}/favorite**

Marks an effect as a favourite or clears the mark.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"favorite":true}' "http://127.0.0.1:9430/api/v1/effects/<id>/favorite"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>/favorite", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"favorite":true}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `SetFavorite`.

| Field | Type | What it holds |
| --- | --- | --- |
| `favorite` | `boolean` | True marks the effect as a favourite. |

**Answer body**: `JSON object`.

The fields the daemon sends: the effect that was marked and whether it is a favourite now.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the mark was saved. |
| `id` | `string` | The effect that was marked. |
| `favorite` | `boolean` | True when the effect is a favourite now. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the effect to mark or unmark. |

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

**PUT**

Marks an effect as a favourite or clears the mark.

```json
{
  "favorite": true
}
```
