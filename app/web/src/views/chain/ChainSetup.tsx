"use client";

// The chain setup flow: three steps that tell the app what is plugged into one header.
//
// The screen is a small flat instrument on the case: a printed bar, the reason the flow exists in a
// recess, one stage of fixed height that holds whichever screen is on, and a foot whose keys keep
// their places while the screen above changes. Everything it shows and every value it edits comes
// from `useChainSetup`; this file only draws it and decides what a key does.
//
// Leaving is the hook's business: Back from the first step and Later both end the session, so the
// header goes back to the length it had, and then this screen closes. Finish and Close close it
// after the commit has already ended the session.

import type { ReactNode } from "react";
import { useState } from "react";
import { CaretLeft, CaretRight, Check, CheckCircle, WarningCircle } from "@phosphor-icons/react";

import { Button } from "@/components/Button";
import type { ChainView } from "@/proto/ChainView";
import { ConfirmStep } from "@/views/chain/ConfirmStep";
import { CountStep } from "@/views/chain/CountStep";
import { LengthStep } from "@/views/chain/LengthStep";
import { chainKey, markChainOffered } from "@/views/chain/offered";
import { useChainSetup } from "@/views/chain/useChainSetup";
import s from "@/views/chain/chain.module.css";

/** Which of the three LEDs in the bar is lit for each screen. */
const STEP_OF = { length: 1, count: 2, confirm: 3, done: 3, error: 1 } as const;

export function ChainSetup({ chain, onClose }: { chain: ChainView; onClose: () => void }) {
  const setup = useChainSetup(chain);
  const [finishing, setFinishing] = useState(false);
  const at = STEP_OF[setup.step];

  /** Yes ends the flow; the key is held while the commit is out, so a second press cannot land. */
  const finish = async () => {
    if (finishing) return;
    setFinishing(true);
    try {
      await setup.confirm();
    } finally {
      setFinishing(false);
    }
  };

  /** Later puts the header back and closes. */
  const later = () => {
    setup.later();
    // The header is remembered: the app does not offer it by itself again, and the owner said
    // "not now" about it. The record is the header's own, so it survives the window (#380).
    markChainOffered(chainKey(chain.device_id, chain.zone_index));
    onClose();
  };

  /** Back from the first step leaves the flow; on the later steps it keeps what was entered. */
  const back = () => {
    setup.back();
    if (setup.step === "length") onClose();
  };

  /** An empty header is written once, then the flow has nothing left to show. */
  const markEmpty = async () => {
    await setup.nothingLitUp();
    onClose();
  };

  const screen = (): ReactNode => {
    switch (setup.step) {
      case "length":
        return (
          <LengthStep
            value={setup.length}
            max={chain.max_leds}
            capped={setup.capped}
            onChange={setup.setLength}
            onNothing={() => void markEmpty()}
          />
        );
      case "count":
        return (
          <CountStep
            length={setup.length}
            count={setup.count}
            parts={setup.parts}
            onCount={setup.setCount}
          />
        );
      case "confirm":
        return (
          <ConfirmStep
            parts={setup.parts}
            length={setup.length}
            max={chain.max_leds}
            onName={setup.setPartName}
            onKind={setup.setPartKind}
            onLeds={setup.setPartLeds}
          />
        );
      case "done":
        return (
          <div className={s.done}>
            <CheckCircle className={s.doneMark} size={28} weight="fill" aria-hidden="true" />
            <p className={s.doneText}>{setup.foundSummary ?? "The header is set up."}</p>
          </div>
        );
      case "error":
        return (
          <div className={s.errorScreen}>
            <h3 className={s.prompt}>The header did not answer.</h3>
            <div className={s.error} role="alert">
              <WarningCircle className={s.errorIcon} size={16} weight="fill" aria-hidden="true" />
              <p className={s.errorText}>{setup.error}</p>
            </div>
          </div>
        );
    }
  };

  const foot = (): ReactNode => {
    if (setup.step === "done") {
      return (
        <div className={s.footEnd}>
          <Button variant="primary" onClick={onClose}>
            Close
          </Button>
        </div>
      );
    }
    if (setup.step === "error") {
      return (
        <>
          <div className={s.footStart}>
            <Button variant="ghost" onClick={later}>
              Later
            </Button>
          </div>
          <div className={s.footEnd}>
            <Button variant="primary" onClick={setup.retry}>
              Retry
            </Button>
          </div>
        </>
      );
    }
    const confirming = setup.step === "confirm";
    return (
      <>
        <div className={s.footStart}>
          <Button variant="ghost" icon={<CaretLeft size={16} aria-hidden="true" />} onClick={back}>
            Back
          </Button>
        </div>
        <div className={s.footEnd}>
          <Button variant="ghost" onClick={later}>
            Later
          </Button>
          <Button
            variant="primary"
            icon={confirming ? <Check size={16} aria-hidden="true" /> : <CaretRight size={16} aria-hidden="true" />}
            busy={confirming && finishing}
            onClick={() => (confirming ? void finish() : setup.next())}
          >
            {confirming ? "Yes" : "Next"}
          </Button>
        </div>
      </>
    );
  };

  return (
    <div className={s.root} role="region" aria-label="Chain setup">
      <div className={s.frame}>
        <header className={s.bar}>
          <div className={s.who}>
            <span className={s.kicker}>Chain setup</span>
            <span className={s.zone}>
              {chain.device_name} · {chain.zone_name}
            </span>
          </div>
          <span className={s.progress} role="img" aria-label={`Step ${at} of 3`}>
            {[1, 2, 3].map((n) => (
              <span key={n} className={s.dot} data-on={n <= at ? "true" : "false"} data-now={n === at ? "true" : "false"} />
            ))}
          </span>
        </header>
        <p className={s.why}>The app cannot see what is plugged into a header.</p>
        <div className={s.chainStage}>
          {setup.error !== null && setup.step !== "error" ? (
            <div className={s.error} role="alert">
              <WarningCircle className={s.errorIcon} size={16} weight="fill" aria-hidden="true" />
              <p className={s.errorText}>{setup.error}</p>
              <Button size="sm" onClick={setup.retry}>
                Retry
              </Button>
            </div>
          ) : null}
          {screen()}
        </div>
        <footer className={s.foot}>{foot()}</footer>
      </div>
    </div>
  );
}
