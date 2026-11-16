"use client";

// The open group: what it plays and how it is tuned.
//
// The sheet is drawn over the page, so opening it moves nothing under the pointer. On a phone it
// is the whole window, with a back key; on a wide window it is a panel at the right edge.
//
// It has four steps. The group itself: the live lights, the effect it plays (and the effect's
// own controls, the same component the Effects page uses), the mode, the brightness, its presets
// and its devices. The name: what a group that was just made is called. The effect: the picker.
// The devices: every light the editor knows, grouped by where it is now.
//
// Every change reaches the lights at once and is on the undo stack; there is no Save and no
// Revert.

import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, CopySimple, DotsThree, Moon, PencilSimple, Plus, Sparkle, Trash, X } from "@phosphor-icons/react";
import { Button, ButtonLink, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { useMediaQuery } from "@/components/MediaQuery";
import { NameField } from "@/components/NameField";
import { Popover } from "@/components/Popover";
import { Segmented } from "@/components/Segmented";
import { Slider } from "@/components/Slider";
import { useFocusTrap } from "@/components/useFocusTrap";
import { api } from "@/lib/endpoints";
import { fillDefaults } from "@/lib/presetMatch";
import { effectNameOf } from "@/lib/status";
import { useSnap } from "@/lib/store";
import { AddDevices } from "@/views/scenes/AddDevices";
import { EffectPicker, isPatternEffect, useEffectDetail } from "@/views/scenes/EffectPicker";
import { GroupStage } from "@/views/scenes/GroupStage";
import { LedRun, runLabel } from "@/views/scenes/LedRun";
import { MemberList } from "@/views/scenes/MemberList";
import { MoveStep } from "@/views/scenes/MoveStep";
import { groupGoneMessage, groupOrderMoved } from "@/views/scenes/groupEdit";
import { PropertyControls } from "@/views/PropertyControls";
import { notify } from "@/components/Notify";
import type { GroupMode } from "@/proto/GroupMode";
import type { LightView } from "@/proto/LightView";
import type { Preset } from "@/proto/Preset";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { Props } from "@/proto/props";
import type { UseScene } from "@/views/scenes/useScene";
import s from "@/views/scenes/scenes.module.css";

/** The two ways a group's lights share its effect, in the owner's words. */
const MODES: { value: GroupMode; label: string }[] = [
  { value: "flow_across", label: "flow across" },
  { value: "each_its_own", label: "each its own" },
];

export interface GroupSheetProps {
  groupId: string;
  use: UseScene;
  /** Every light the editor knows. */
  lights: LightView[];
  /** The sheet closed. */
  onClose: () => void;
  /** The sheet opens on this step: the group, its name, the effect or the devices. */
  initial?: "group" | "name" | "effect" | "devices";
}

/**
 * A sheet over the page: a panel at the right edge on a wide window, the whole window on a
 * phone. It is drawn over the page, so nothing under the pointer moves when it opens.
 */
export function Sheet({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const phone = useMediaQuery("(max-width: 720px)");
  const panel = useRef<HTMLElement>(null);
  useFocusTrap({ panelRef: panel, capture: false, onEscape: onClose });
  return (
    <div className={s.sheetRoot}>
      <div className={s.sheetScrim} data-testid="sheet-scrim" onClick={onClose} aria-hidden="true" />
      <section
        className={s.sheet}
        role="dialog"
        aria-label={title}
        data-testid="group-sheet"
        ref={panel}
      >
        <header className={s.sheetHead}>
          <IconButton
            label={phone ? "Back" : "Close"}
            icon={phone ? <ArrowLeft size={18} /> : <X size={18} />}
            onClick={onClose}
          />
          <h2 className={s.sheetTitle}>{title}</h2>
        </header>
        <div className={s.sheetBody}>{children}</div>
      </section>
    </div>
  );
}

/** The sheet's own step: the group, its name, the effect, the devices or the move. */
type Step = "group" | "name" | "effect" | "devices" | "move";

export function GroupSheet({ groupId, use, lights, onClose, initial }: GroupSheetProps) {
  const { edit, setProps, setBrightness, undo } = use;
  const effects = useSnap((st) => st.effects);
  const phone = useMediaQuery("(max-width: 720px)");
  const scene = use.scene;
  const group = scene?.groups.find((g) => g.id === groupId) ?? null;
  const [step, setStep] = useState<Step>(initial ?? "group");
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(group?.name ?? "");
  const [menu, setMenu] = useState(false);
  /** The move step: the group is moved, or copied into the other scene. */
  const [moveKind, setMoveKind] = useState<"move" | "copy">("move");
  const menuAnchor = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLElement>(null);
  /** A group can only go to another scene, and there must be one. */
  const canMove = use.scenes.length > 1;

  // Escape closes the sheet, one step at a time: a step of its own first, then the sheet.
  useFocusTrap({
    panelRef: panel,
    capture: false,
    onEscape: () => {
      if (menu) {
        setMenu(false);
        return;
      }
      if (step !== "group") {
        setStep("group");
        return;
      }
      onClose();
    },
  });

  const effectId = group?.plays.type === "effect" ? group.plays.effect : null;
  // A pattern plays the whole pattern on every device: the group's stored mode is kept, but it is
  // not shown, because there is nothing it could change.
  const pattern = effectId !== null && isPatternEffect(effectId);
  /** The count that asks for the effect's detail again: its presets change under the sheet. */
  const [reload, setReload] = useState(0);
  const effect = useEffectDetail(effectId, reload);
  const values = group?.plays.type === "effect" ? group.plays.props : {};

  const members = useMemo(
    () => (group?.members ?? []).flatMap((id) => lights.filter((l) => l.light.id === id)),
    [group?.members, lights],
  );

  // The stage's own facts (#378):
  // - the daemon holds the group in its saved scene: a group that is still a draft is
  //   answered with 404, so the stage waits for the save instead of opening a socket.
  const scenes = useSnap((st) => st.scenes);
  const activeId = scenes?.active ?? "";
  const savedScene = scenes?.scenes.find((s) => s.id === activeId);
  const saved = savedScene?.groups.some((g) => g.id === groupId) ?? false;
  // - the lights are paused: the engine sends nothing at all.
  const paused = useSnap((st) => st.status?.power === "paused");
  // - why the group is dark, from the daemon.
  const failed = use.states.find((s) => s.id === groupId)?.failed ?? null;
  // - the devices of the group that are not connected.
  const away = members.filter((l) => !l.connected).length;

  /** The group's name: the edit is an ordinary change, so an Undo takes it back. */
  const commitName = useCallback(
    (next: string) => {
      const clean = next.trim();
      setRenaming(false);
      if (!clean || clean === group?.name) return;
      edit((draft) => {
        const g = draft.groups.find((x) => x.id === groupId);
        if (g) g.name = clean;
      }, "Rename the group");
    },
    [edit, group?.name, groupId, setRenaming],
  );

  /** The group goes: the scene keeps its other groups. */
  const deleteGroup = useCallback(() => {
    edit((draft) => {
      draft.groups = draft.groups.filter((g) => g.id !== groupId);
    }, "Delete the group");
    onClose();
  }, [edit, groupId, onClose]);

  /** The groups of the scene, in their order, and where this one is in it. */
  const groupIds = (scene?.groups ?? []).map((g) => g.id);
  const groupAt = groupIds.indexOf(groupId);

  /**
   * The group one place up or down in the scene (#384). This is the phone's path: the strip
   * scrolls sideways there, and the rows are dragged by a handle that a finger does not drag.
   */
  const moveGroup = useCallback(
    (delta: number) => {
      const order = groupOrderMoved(groupIds, groupId, delta);
      if (!order) return;
      use.reorderGroups(order);
    },
    [groupIds, groupId, use],
  );

  /** A group that was left with no devices: the page says so and offers the Undo. */
  const emptied = useCallback(
    (names: string[]) => {
      notify(groupGoneMessage(names), "warn", { label: "Undo", onClick: () => undo() });
    },
    [undo],
  );

  if (!group) return null;

  const effectName = effectId ? effectNameOf(effects, effectId) : "Off";
  const title =
    step === "effect"
      ? "What it plays"
      : step === "devices"
        ? "Add devices"
        : step === "move"
          ? `${moveKind === "copy" ? "Copy" : "Move"} ${group.name}`
          : group.name;

  return (
    <div className={s.sheetRoot}>
      <div className={s.sheetScrim} data-testid="sheet-scrim" onClick={onClose} aria-hidden="true" />
      <section
        className={s.sheet}
        role="dialog"
        aria-label={title}
        data-testid="group-sheet"
        data-step={step}
        ref={panel}
      >
        <header className={s.sheetHead}>
          <IconButton
            label={step === "group" ? (phone ? "Back" : "Close") : "Back"}
            icon={step === "group" && !phone ? <X size={18} /> : <ArrowLeft size={18} />}
            onClick={() => (step === "group" ? onClose() : setStep("group"))}
          />
          {renaming ? (
            <NameField
              value={name}
              label="The name of this group"
              placeholder="Group name"
              onChange={setName}
              onEnter={() => commitName(name)}
              onEscape={() => setRenaming(false)}
            />
          ) : (
            <h2 className={s.sheetTitle}>{title}</h2>
          )}
          <Button
            ref={menuAnchor}
            className={s.iconKey}
            aria-label="More about this group"
            aria-haspopup="true"
            aria-expanded={menu}
            icon={<DotsThree size={18} weight="bold" />}
            onClick={() => setMenu((v) => !v)}
          />
          <Popover
            anchor={menuAnchor}
            open={menu}
            onClose={() => setMenu(false)}
            label="More about this group"
            className={s.sheetMenu}
          >
            <Button
              variant="ghost"
              size="sm"
              icon={<PencilSimple size={16} />}
              onClick={() => {
                setMenu(false);
                setName(group.name);
                setRenaming(true);
              }}
            >
              Rename
            </Button>
            {effectId && (
              <ButtonLink
                href={`/effects/?effect=${encodeURIComponent(effectId)}&group=${encodeURIComponent(groupId)}`}
                variant="ghost"
                size="sm"
                icon={<Sparkle size={16} />}
                onClick={() => setMenu(false)}
              >
                Edit effect
              </ButtonLink>
            )}
            <Button
              variant="ghost"
              size="sm"
              icon={<ArrowRight size={16} />}
              disabled={!canMove}
              title={canMove ? undefined : "There is no other scene."}
              onClick={() => {
                setMenu(false);
                setMoveKind("move");
                setStep("move");
              }}
            >
              Move to scene
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<CopySimple size={16} />}
              disabled={!canMove}
              title={canMove ? undefined : "There is no other scene."}
              onClick={() => {
                setMenu(false);
                setMoveKind("copy");
                setStep("move");
              }}
            >
              Copy to scene
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<ArrowUp size={16} />}
              disabled={groupAt <= 0}
              title={groupAt <= 0 ? "It is already first." : undefined}
              onClick={() => {
                setMenu(false);
                moveGroup(-1);
              }}
            >
              Move up
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<ArrowDown size={16} />}
              disabled={groupAt < 0 || groupAt >= groupIds.length - 1}
              title={groupAt >= 0 && groupAt >= groupIds.length - 1 ? "It is already last." : undefined}
              onClick={() => {
                setMenu(false);
                moveGroup(1);
              }}
            >
              Move down
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<Trash size={16} />}
              onClick={() => {
                setMenu(false);
                deleteGroup();
              }}
            >
              Delete group
            </Button>
          </Popover>
        </header>

        <div className={s.sheetBody}>
          {step === "name" && (
            <div className={s.sheetStep}>
              <div className={s.sheetRun}>
                <LedRun lights={members} label={runLabel(group.name, members)} />
              </div>
              <p className={s.sheetLead}>What is this group called?</p>
              <NameField
                value={name}
                label="The name of this group"
                placeholder="Group name"
                onChange={setName}
                onEnter={() => {
                  commitName(name);
                  setStep("effect");
                }}
                onEscape={() => setStep("group")}
              />
              <Button
                variant="primary"
                size="sm"
                className={s.sheetNext}
                onClick={() => {
                  commitName(name);
                  setStep("effect");
                }}
              >
                Next
              </Button>
            </div>
          )}

          {step === "effect" && (
            <div className={s.sheetStep}>
              <EffectPicker groupId={groupId} use={use} onClose={() => setStep("group")} />
            </div>
          )}

          {step === "devices" && (
            <div className={s.sheetStep}>
              <AddDevices
                groupId={groupId}
                lights={lights}
                use={use}
                onClose={() => setStep("group")}
                onEmptied={emptied}
              />
            </div>
          )}

          {step === "move" && (
            <div className={s.sheetStep}>
              <MoveStep
                groupId={groupId}
                copy={moveKind === "copy"}
                use={use}
                lights={lights}
                onClose={onClose}
              />
            </div>
          )}

          {step === "group" && (
            <>
              {/* The stage (#378): the picture the group itself draws, and the group's own
                  controls beside it. The LED run stays under it, in the flow. */}
              <GroupStage
                group={group}
                effectName={effectName}
                failed={failed}
                pattern={pattern}
                saved={saved}
                paused={paused}
                away={away}
              >
                <Button
                  className={s.sheetEffect}
                  data-testid="sheet-effect"
                  icon={effectId ? <Sparkle size={16} /> : <Moon size={16} />}
                  onClick={() => setStep("effect")}
                >
                  <span className="one-line">{effectName}</span>
                  {pattern && <Chip>Pattern</Chip>}
                </Button>
                {!pattern && (
                  <Segmented<GroupMode>
                    label="How this group's lights share the effect"
                    value={group.mode}
                    options={MODES}
                    onChange={(mode) =>
                      edit((draft) => {
                        const g = draft.groups.find((x) => x.id === groupId);
                        if (g) g.mode = mode;
                      }, "Change how the lights share the effect")
                    }
                  />
                )}
              </GroupStage>

              <div className={s.sheetRun}>
                <LedRun lights={members} label={runLabel(group.name, members)} />
              </div>

              <div className={s.sheetFader}>
                <Slider
                  label={`${group.name} brightness`}
                  min={0}
                  max={100}
                  value={group.brightness}
                  onChange={(n) => setBrightness(groupId, n)}
                />
                <span className={`${s.sheetPct} fig`}>{group.brightness}%</span>
              </div>

              {/* A pattern has no settings and no presets (0.16.0): only a canvas effect has them. */}
              {effectId && effect && !pattern && (
                <section className={s.sheetBlock}>
                  <h3 className={s.sheetBlockTitle}>The effect</h3>
                  <PropertyControls
                    schema={effect.schema}
                    values={values}
                    onChange={(key, value) => setProps(groupId, { [key]: value })}
                  />
                </section>
              )}

              {effectId && effect && !pattern && (
                <Presets
                  effectId={effectId}
                  builtIn={effect.schema.presets}
                  user={effect.user_presets}
                  values={values}
                  defs={effect.schema.properties}
                  onApply={(preset) =>
                    setProps(groupId, fillDefaults(effect.schema.properties, preset.props))
                  }
                  onSaved={() => setReload((n) => n + 1)}
                />
              )}

              <section className={s.sheetBlock}>
                <div className={s.sheetBlockHead}>
                  <h3 className={s.sheetBlockTitle}>The devices</h3>
                  <Button
                    size="sm"
                    data-testid="sheet-add-devices"
                    icon={<Plus size={14} />}
                    onClick={() => setStep("devices")}
                  >
                    Add devices
                  </Button>
                </div>
                <MemberList groupId={groupId} lights={lights} use={use} />
              </section>
            </>
          )}
        </div>
      </section>
    </div>
  );
}

/**
 * The effect's presets: the built-in ones and the owner's, then "Save as preset". Choosing one
 * copies its values into the group; saving one writes a preset of the effect.
 */
function Presets({
  effectId,
  builtIn,
  user,
  values,
  defs,
  onApply,
  onSaved,
}: {
  effectId: string;
  builtIn: Preset[];
  user: Preset[];
  values: Props;
  defs: PropertyDef[];
  onApply: (preset: Preset) => void;
  onSaved: () => void;
}) {
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const presets = [...builtIn, ...user];

  const save = useCallback(async () => {
    const clean = name.trim();
    if (!clean) {
      setError("Give the preset a name.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.createPreset(effectId, { name: clean, props: fillDefaults(defs, values) });
      setNaming(false);
      setName("");
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : "The preset could not be saved.");
    } finally {
      setBusy(false);
    }
  }, [defs, effectId, name, onSaved, values]);

  return (
    <section className={s.sheetBlock}>
      <div className={s.sheetBlockHead}>
        <h3 className={s.sheetBlockTitle}>Presets</h3>
        {!naming && (
          <Button size="sm" onClick={() => setNaming(true)}>
            Save as preset
          </Button>
        )}
      </div>
      <div className={s.presetRow} role="group" aria-label="The presets of this effect">
        {presets.map((preset) => (
          <Chip key={preset.name} label={`Use the preset ${preset.name}`} onClick={() => onApply(preset)}>
            <span className="one-line">{preset.name}</span>
          </Chip>
        ))}
        {presets.length === 0 && <span className={s.presetNone}>This effect has no presets yet.</span>}
      </div>
      {naming && (
        <div className={s.presetName}>
          <NameField
            value={name}
            label="The name of the new preset"
            placeholder="Name these values"
            invalid={error !== null}
            onChange={setName}
            onEnter={() => void save()}
            onEscape={() => {
              setNaming(false);
              setError(null);
            }}
          />
          <Button variant="primary" size="sm" busy={busy} onClick={() => void save()}>
            Save preset
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setNaming(false);
              setError(null);
            }}
          >
            Cancel
          </Button>
        </div>
      )}
      {error && (
        <p className={s.presetError} role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
