"use client";

// The header setup flow: the screens that tell the app what is plugged into one ARGB header.
//
// The screen is a small flat instrument on the case: a printed bar, the labelled steps, the reason
// the flow exists in a recess, one stage of fixed height that holds whichever screen is on, and a
// foot whose three slots (Back, Later, the primary key) keep their places on every screen. A slot
// with no key stays empty, so the keys never move when the screen changes. Everything it shows and
// every value it edits comes from `useChainSetup`; this file only draws it and decides what a key
// does.
//
// Leaving is the hook's business: Back from the first question and Later both end the session, so
// the header goes back to the length it had, and then this screen closes. Done closes it after the
// commit has already ended the session.

import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { CaretLeft, CaretRight, Check, CheckCircle, WarningCircle } from "@phosphor-icons/react";

import { Button } from "@/components/Button";
import type { ChainView } from "@/proto/ChainView";
import { ConfirmStep } from "@/views/chain/ConfirmStep";
import { CountStep } from "@/views/chain/CountStep";
import { IntroStep } from "@/views/chain/IntroStep";
import { LengthStep } from "@/views/chain/LengthStep";
import { LitCountStep } from "@/views/chain/LitCountStep";
import { MirrorCheck } from "@/views/chain/MirrorCheck";
import { BlindStep, NothingLitPanel } from "@/views/chain/NothingLitPanel";
import { StartingStep } from "@/views/chain/StartingStep";
import { chainKey, markChainOffered } from "@/views/chain/offered";
import { useChainSetup, type ChainStep, type IntroKind } from "@/views/chain/useChainSetup";
import s from "@/views/chain/chain.module.css";

/** The labels under the bar: a series counts devices, a mirror does not. */
const SERIES_STEPS = ["Length", "Count", "Check"] as const;
const MIRROR_STEPS = ["Length", "Check"] as const;

/**
 * How many labels are lit for a screen. Nothing is lit before the first question, the first label
 * is lit while the lights are counted and slid, and all of them are lit when the flow is done.
 */
function litLabels(step: ChainStep, mirror: boolean, last: number): number {
  switch (step) {
    case "intro":
    case "starting":
      return 0;
    case "lit":
    case "length":
    case "nothing":
    case "blind":
      return 1;
    case "count":
      return 2;
    case "check":
      return mirror ? 2 : 3;
    case "done":
      return last;
    case "error":
      return 0;
  }
}

export interface ChainSetupProps {
  chain: ChainView;
  onClose: () => void;
  /**
   * What the flow opens on. `full` is the introduction (the first header of a visit), `next` the
   * short card for a later header, `none` no introduction: the session opens at once (a flow opened
   * from a device's own row).
   */
  intro?: IntroKind;
  /** The header before this one, when the owner marked it empty a moment ago. */
  previousEmpty?: string | null;
  /**
   * True when the visit has more headers to show after this one. A header marked empty then closes
   * at once and the next one opens; otherwise the screen says it is marked empty.
   */
  more?: boolean;
}

export function ChainSetup({ chain, onClose, intro = "full", previousEmpty = null, more = false }: ChainSetupProps) {
  const setup = useChainSetup(chain, intro);
  const [finishing, setFinishing] = useState(false);
  /** A ref as well as the state: two presses in one tick both read the state as it was rendered. */
  const finishingNow = useRef(false);
  const [marking, setMarking] = useState(false);
  const markingNow = useRef(false);
  const [retrying, setRetrying] = useState(false);
  const retryingNow = useRef(false);
  const zone = chain.zone_name;
  const labels = setup.mirror ? MIRROR_STEPS : SERIES_STEPS;
  const lit = litLabels(setup.step, setup.mirror, labels.length);

  // A setup that is on screen is that header's one offer, wherever it was opened from: the
  // takeover, or a device's own row. Without the mark a header put off from its row would be
  // offered again by itself on the next visit to Scenes (#380).
  useEffect(() => {
    markChainOffered(chainKey(chain.device_id, chain.zone_index));
  }, [chain.device_id, chain.zone_index]);

  // A header marked empty while more headers wait: the next one opens at once.
  const emptied = setup.step === "done" && setup.doneKind === "empty";
  useEffect(() => {
    if (emptied && more) onClose();
  }, [emptied, more, onClose]);

  /** Save ends the flow; the key is held while the commit is out, so a second press cannot land. */
  const finish = async () => {
    if (finishingNow.current) return;
    finishingNow.current = true;
    setFinishing(true);
    try {
      await setup.confirm();
    } finally {
      finishingNow.current = false;
      setFinishing(false);
    }
  };

  /**
   * Retry sends again what failed: a commit, the empty call, or a whole new session. The key is
   * held while its call is out, and the hook drops a second one as well.
   */
  const retry = async () => {
    if (retryingNow.current) return;
    retryingNow.current = true;
    setRetrying(true);
    try {
      await setup.retry();
    } finally {
      retryingNow.current = false;
      setRetrying(false);
    }
  };

  /**
   * Mark empty writes the header's record once. The key is held while the call is out, and the hook
   * drops a second call as well: a second press cannot land.
   */
  const markEmpty = async () => {
    // A ref, not the state: two presses inside one tick both read the state as it was rendered,
    // and the second one's finish would let go of the key while the first call is still out.
    if (markingNow.current) return;
    markingNow.current = true;
    setMarking(true);
    try {
      await setup.markEmpty();
    } finally {
      markingNow.current = false;
      setMarking(false);
    }
  };

  /**
   * Later puts the header back and closes. The header was marked as offered when this screen came
   * up; the headers the visit has not shown yet are the takeover's to put off.
   */
  const later = () => {
    setup.later();
    onClose();
  };

  /** Back from the first question leaves the flow; on the later screens it keeps what was entered. */
  const back = () => {
    const leaving = setup.step === "lit";
    setup.back();
    if (leaving) onClose();
  };

  const screen = (): ReactNode => {
    switch (setup.step) {
      case "intro":
        return (
          <IntroStep
            kind={intro === "next" ? "next" : "full"}
            zone={zone}
            previousEmpty={previousEmpty}
            onEmpty={setup.startEmpty}
          />
        );
      case "starting":
        return <StartingStep zone={zone} intent={setup.intent} />;
      case "lit":
        return (
          <LitCountStep
            copies={setup.copies}
            onCopies={setup.setCopies}
            onNothing={setup.noLight}
            onBlind={setup.cantSee}
          />
        );
      case "length":
        return (
          <LengthStep
            value={setup.length}
            max={chain.max_leds}
            capped={setup.capped}
            copies={setup.copies}
            onChange={setup.setLength}
          />
        );
      case "count":
        return (
          <CountStep
            zone={zone}
            length={setup.length}
            count={setup.count}
            parts={setup.parts}
            onCount={setup.setCount}
            onCountAgain={setup.countAgain}
          />
        );
      case "check":
        return setup.mirror ? (
          <MirrorCheck
            zone={zone}
            length={setup.length}
            copies={setup.copies}
            name={setup.mirrorName}
            onName={setup.setMirrorName}
          />
        ) : (
          <ConfirmStep
            parts={setup.parts}
            length={setup.length}
            max={chain.max_leds}
            onName={setup.setPartName}
            onKind={setup.setPartKind}
            onLeds={setup.setPartLeds}
          />
        );
      case "nothing":
        return (
          <NothingLitPanel
            zone={zone}
            busy={marking}
            onBlind={setup.cantSee}
            onEmpty={() => void markEmpty()}
          />
        );
      case "blind":
        return <BlindStep />;
      case "done":
        return (
          <div className={s.done}>
            <CheckCircle className={s.doneMark} size={28} weight="fill" aria-hidden="true" />
            <p className={s.doneText}>{setup.doneText}</p>
            {setup.doneNote ? <p className={s.second}>{setup.doneNote}</p> : null}
          </div>
        );
      case "error":
        return (
          <div className={s.errorScreen}>
            <h3 className={s.prompt}>{zone} did not answer.</h3>
            <div className={s.error} role="alert">
              <WarningCircle className={s.errorIcon} size={16} weight="fill" aria-hidden="true" />
              <p className={s.errorText}>{setup.error}</p>
            </div>
          </div>
        );
    }
  };

  /** The foot: which key sits in each of its three slots on this screen. */
  const foot = (): { back: ReactNode; later: ReactNode; primary: ReactNode } => {
    const laterKey = (
      <Button variant="ghost" onClick={later}>
        Later
      </Button>
    );
    const backKey = (
      <Button variant="ghost" icon={<CaretLeft size={16} aria-hidden="true" />} onClick={back}>
        Back
      </Button>
    );
    switch (setup.step) {
      case "intro":
        return {
          back: null,
          later: laterKey,
          primary: (
            <Button variant="primary" onClick={setup.start}>
              Start
            </Button>
          ),
        };
      case "starting":
        return { back: null, later: laterKey, primary: null };
      case "lit":
      case "length":
      case "count":
        return {
          back: backKey,
          later: laterKey,
          primary: (
            <Button variant="primary" icon={<CaretRight size={16} aria-hidden="true" />} onClick={setup.next}>
              Next
            </Button>
          ),
        };
      case "check":
        return {
          back: backKey,
          later: laterKey,
          primary: (
            <Button
              variant="primary"
              icon={<Check size={16} aria-hidden="true" />}
              busy={finishing}
              onClick={() => void finish()}
            >
              {setup.mirror ? "Save" : "Yes, save these devices"}
            </Button>
          ),
        };
      case "nothing":
      case "blind":
        return {
          back: null,
          later: laterKey,
          primary: (
            <Button variant="primary" onClick={setup.tryAgain}>
              Try again
            </Button>
          ),
        };
      case "done":
        return {
          back: null,
          later: null,
          primary: (
            <Button variant="primary" onClick={onClose}>
              Done
            </Button>
          ),
        };
      case "error":
        return {
          back: null,
          later: laterKey,
          primary: (
            <Button variant="primary" busy={retrying} onClick={() => void retry()}>
              Retry
            </Button>
          ),
        };
    }
  };

  const keys = foot();

  return (
    <div className={s.root} role="region" aria-label="Light setup">
      <div className={s.frame}>
        <header className={s.bar}>
          <div className={s.who}>
            <span className={s.kicker}>Light setup</span>
            <span className={s.zone}>
              {chain.device_name} · {zone}
            </span>
          </div>
        </header>
        <ol className={s.steps} aria-label="Steps">
          {labels.map((label, i) => (
            <li
              key={label}
              className={s.stepLabel}
              data-on={i < lit ? "true" : "false"}
              data-now={i === lit - 1 ? "true" : "false"}
              aria-current={i === lit - 1 ? "step" : undefined}
            >
              <span className={s.stepDot} aria-hidden="true" />
              {label}
            </li>
          ))}
        </ol>
        <p className={s.why}>The app cannot see what is plugged into this ARGB header.</p>
        <div className={s.chainStage}>
          {setup.error !== null && setup.step !== "error" ? (
            <div className={s.error} role="alert">
              <WarningCircle className={s.errorIcon} size={16} weight="fill" aria-hidden="true" />
              <p className={s.errorText}>{setup.error}</p>
              <Button size="sm" busy={retrying} onClick={() => void retry()}>
                Retry
              </Button>
            </div>
          ) : null}
          {screen()}
        </div>
        <footer className={s.foot}>
          <div className={s.footBack}>{keys.back}</div>
          <div className={s.footLater}>{keys.later}</div>
          <div className={s.footPrimary}>{keys.primary}</div>
        </footer>
      </div>
    </div>
  );
}
