# Effect defaults

Method and path: PATCH /api/v1/effects/{id}/defaults.

Area: Effects.

## Request

**PATCH /api/v1/effects/{id}/defaults**

Changes the starting property values of an effect.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"speed":{"type":"number","value":30}}' "http://127.0.0.1:9430/api/v1/effects/<id>/defaults"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>/defaults", {
  method: "PATCH",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"speed":{"type":"number","value":30}}),
});
const data = await response.json();
```

## Types

**PATCH**

**Request body**: `Props`.

A map of property id to PropValue.

**Answer body**: `JSON object`.

The fields the daemon sends: the effect whose starting values changed and the values they now hold.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the starting values were saved. |
| `id` | `string` | The effect whose starting values changed. |
| `props` | `JSON object` | The merged starting values. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the effect whose starting values change. |

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

**PATCH**

Changes the starting property values of an effect.

```json
{
  "speed": {
    "type": "number",
    "value": 30
  }
}
```
