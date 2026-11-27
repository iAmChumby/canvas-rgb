# API reference

## Start here

The daemon listens on 127.0.0.1:9430. It binds loopback only. Every route lives under /api/v1. The port is fixed at 9430.
The editor drives the lights through this API, and so do the CLI and the tray. The routes report the daemon's state and drive the devices, fans, effects and scenes. Other routes read displays and peripherals, change settings, install updates, export or import configuration, run diagnostics, handle first-run onboarding and stop the daemon. The reference gives the request and answer for each route.

## Routes by area

### Status and output

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/about` | none | About |
| GET | `/api/v1/about/report` | none | AboutReport |
| GET | `/api/v1/output` | none | OutputState |
| PATCH | `/api/v1/output` | PatchOutput | OutputState |
| POST | `/api/v1/quit` | none | JSON object |
| GET | `/api/v1/status` | none | Status |

### Devices and lights

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/devices` | none | DevicesResponse |
| POST | `/api/v1/devices/{id}/identify` | IdentifyRequest | JSON object |
| PUT | `/api/v1/devices/{id}/white-balance` | WhiteBalance | WhiteBalances |
| DELETE | `/api/v1/devices/{id}/white-balance` | none | WhiteBalances |
| PATCH | `/api/v1/devices/{id}/zones/{zone}` | PatchZone | JSON object |
| GET | `/api/v1/lights` | none | LightsResponse |
| PATCH | `/api/v1/lights/{id}` | LightPatch | LightView |
| DELETE | `/api/v1/lights/{id}` | none | LightDeleted |
| POST | `/api/v1/lights/flash` | FlashRequest | JSON object |
| GET | `/api/v1/white-balance` | none | WhiteBalances |

### Effects

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/effects` | none | EffectsResponse |
| GET | `/api/v1/effects/{id}` | none | EffectDetail |
| PUT | `/api/v1/effects/{id}` | EffectDetail | JSON object |
| DELETE | `/api/v1/effects/{id}` | none | DeleteEffectResponse |
| POST | `/api/v1/effects/{id}/activate` | ActivateEffect | JSON object |
| PATCH | `/api/v1/effects/{id}/defaults` | Props | JSON object |
| PUT | `/api/v1/effects/{id}/favorite` | SetFavorite | JSON object |
| POST | `/api/v1/effects/{id}/presets` | PresetCreate | PresetsResponse |
| PUT | `/api/v1/effects/{id}/presets/{name}` | PresetReplace | PresetsResponse |
| DELETE | `/api/v1/effects/{id}/presets/{name}` | none | PresetsResponse |
| GET | `/api/v1/effects/{id}/source` | none | EffectSource |
| GET | `/api/v1/effects/{id}/thumbnail` | none | Binary thumbnail |
| GET | `/api/v1/effects/draft` | none | DraftStatus |
| PUT | `/api/v1/effects/draft` | DraftRequest | DraftStatus |
| DELETE | `/api/v1/effects/draft` | none | DraftStatus |
| POST | `/api/v1/effects/import` | ImportEffect | ImportEffectResponse |
| POST | `/api/v1/effects/preview` | EffectPreviewRequest | EffectPreviewResponse |

### Stacks and patterns

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/patterns` | none | PatternsResponse |
| GET | `/api/v1/patterns/{id}` | none | Pattern |
| PUT | `/api/v1/patterns/{id}` | Pattern | Pattern |
| DELETE | `/api/v1/patterns/{id}` | none | DeletePatternResponse |
| GET | `/api/v1/stacks` | none | StacksResponse |
| GET | `/api/v1/stacks/{id}` | none | StackDetail |
| PUT | `/api/v1/stacks/{id}` | Stack | Stack |
| DELETE | `/api/v1/stacks/{id}` | none | DeleteStackResponse |
| PUT | `/api/v1/stacks/preview` | StackPreviewRequest | StackPreviewResponse |
| DELETE | `/api/v1/stacks/preview` | none | JSON object |

### Fans

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/fans` | none | FansStatus |
| PATCH | `/api/v1/fans/headers/{h}` | FanMode | JSON object |
| PUT | `/api/v1/fans/headers/{h}/availability` | PutFanHeaderAvailability | JSON object |
| PUT | `/api/v1/fans/hot` | HotLimits | FansStatus |
| GET | `/api/v1/fans/profile` | none | FanProfileName |
| PATCH | `/api/v1/fans/profile` | FanProfileName | FanProfileName |
| PUT | `/api/v1/fans/profiles/{name}` | FanProfile | FanProfile |
| DELETE | `/api/v1/fans/profiles/{name}` | none | JSON object |

### Scenes and chains

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/chains` | none | ChainsResponse |
| POST | `/api/v1/chains/{device}/{zone}/setup` | none | SetupStarted |
| DELETE | `/api/v1/chains/{device}/{zone}/setup` | SetupEmpty | JSON object |
| POST | `/api/v1/chains/{device}/{zone}/setup/commit` | SetupCommit | SetupCommitted |
| POST | `/api/v1/chains/{device}/{zone}/setup/empty` | SetupEmpty | JSON object |
| POST | `/api/v1/chains/{device}/{zone}/setup/light` | SetupLight | JSON object |
| GET | `/api/v1/scenes` | none | ScenesResponse |
| POST | `/api/v1/scenes` | SceneCreate | SceneAnswer |
| PUT | `/api/v1/scenes` | ScenesWrite | ScenesResponse |
| PUT | `/api/v1/scenes/{id}` | ScenePut | SceneAnswer |
| DELETE | `/api/v1/scenes/{id}` | none | ScenesResponse |
| PUT | `/api/v1/scenes/{id}/groups/{group}/brightness` | GroupBrightnessPut | Scene |
| POST | `/api/v1/scenes/{id}/groups/{group}/move` | GroupMove | GroupMoved |
| PATCH | `/api/v1/scenes/{id}/groups/{group}/props` | Props | Scene |
| PUT | `/api/v1/scenes/active` | SceneActivePut | ScenesResponse |

### Audio

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| PUT | `/api/v1/audio/source` | AudioSource | ResolvedAudioSource |
| GET | `/api/v1/audio/sources` | none | AudioSources |
| GET | `/api/v1/audio/test` | none | AudioTestState |
| PUT | `/api/v1/audio/test` | AudioTestRequest | AudioTestLease |
| DELETE | `/api/v1/audio/test` | none | JSON object |

### Preview

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/preview` | none | WebSocket (binary frames) |
| GET | `/api/v1/preview/group` | none | WebSocket (binary frames) |
| GET | `/api/v1/preview/lights` | none | WebSocket (binary frames) |

### Displays and peripherals

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/displays` | none | array of DisplayStatus |
| PUT | `/api/v1/displays/{id}` | DisplayConfig | DisplayConfig |
| DELETE | `/api/v1/displays/{id}` | none | JSON object |
| PUT | `/api/v1/displays/{id}/value` | SetDisplayValue | SetDisplayValue |
| GET | `/api/v1/peripherals` | none | PeripheralsResponse |
| GET | `/api/v1/peripherals/{id}` | none | PeripheralDetail |
| PATCH | `/api/v1/peripherals/{id}` | Props | Props |
| PUT | `/api/v1/peripherals/{id}/access` | PeripheralAccess | PeripheralAccess |
| GET | `/api/v1/peripherals/{id}/read` | none | ReadProgress |

### Settings and update

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| GET | `/api/v1/config/export` | none | ExportBundle |
| POST | `/api/v1/config/import` | ExportBundle | JSON object |
| POST | `/api/v1/first-run/dismiss-zone-panel` | none | JSON object |
| POST | `/api/v1/onboarding/complete` | none | JSON object |
| GET | `/api/v1/settings` | none | Settings |
| PUT | `/api/v1/settings` | Settings | Settings |
| POST | `/api/v1/update/install` | InstallUpdateRequest | InstallUpdateResponse |

### Diagnostics

| Method | Path | Body | Answer |
| --- | --- | --- | --- |
| POST | `/api/v1/diag/fail-sensor` | FailSensorRequest | JSON object |
| GET | `/api/v1/diag/readback` | none | ReadbackResponse |

## Guides

- [API getting started](/docs/getting-started): the token, the guards and two short requests.
- [CLI reference](/docs/cli): the same operations from a terminal.
- [Writing an effect](/docs/effects): the file format and the runtime.
