# Activate effect

Method and path: POST /api/v1/effects/{id}/activate.

Area: Effects.

## Request

**POST /api/v1/effects/{id}/activate**

Plays an effect on the groups of the active scene, using saved values or a named preset.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"props":{"speed":{"type":"number","value":30}},"preset":null}' "http://127.0.0.1:9430/api/v1/effects/<id>/activate"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>/activate", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"props":{"speed":{"type":"number","value":30}},"preset":null}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `ActivateEffect`.

| Field | Type | What it holds |
| --- | --- | --- |
| `props` | `an object keyed by text, with PropValue values` | The values that sit on top of the saved values, the defaults or the preset. Optional. |
| `preset` | `string or null` | The name of a built-in preset or a user preset. A preset's values sit on the defaults, so two presets never leak into each other. Optional. |
| `groups` | `array of string` | The groups of the active scene to play the effect on, by name or id. Leave it out to play it on every group, from the effect's starting values. Optional. |

**Answer body**: `JSON object`.

The fields the daemon sends: the effect that was played and the values it played with.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the effect was activated. |
| `id` | `string` | The effect that was played. |
| `props` | `JSON object` | The property values put on the lights. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the effect to play. |

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

Plays an effect on the groups of the active scene, using saved values or a named preset.

```json
{
  "props": {
    "speed": {
      "type": "number",
      "value": 30
    }
  },
  "preset": null
}
```
