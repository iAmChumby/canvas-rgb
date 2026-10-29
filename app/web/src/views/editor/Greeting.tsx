"use client";

import { useState } from "react";
import s from "@/views/editor/onboarding.module.css";

/**
 * The greeting on the first screen: the plain everyday hello in the owner's system language
 * (Luke: "it should just say the default hello"). English, or any language not listed, says
 * "Hello". The trailing dot is the brand accent.
 */
const HELLO: Record<string, string> = {
  en: "Hello",
  es: "Hola",
  fr: "Bonjour",
  de: "Hallo",
  it: "Ciao",
  pt: "Olá",
  nl: "Hallo",
  sv: "Hej",
  da: "Hej",
  nb: "Hei",
  no: "Hei",
  fi: "Hei",
  pl: "Cześć",
  cs: "Ahoj",
  sk: "Ahoj",
  hu: "Szia",
  ro: "Salut",
  tr: "Merhaba",
  el: "Γεια σας",
  ru: "Привет",
  uk: "Привіт",
  ar: "مرحبا",
  he: "שלום",
  hi: "नमस्ते",
  ja: "こんにちは",
  ko: "안녕하세요",
  zh: "你好",
  vi: "Xin chào",
  th: "สวัสดี",
  id: "Halo",
};

/** The hello for a BCP 47 tag such as "pt-BR": matched on the language part only. */
export function helloFor(tag: string | undefined): string {
  const lang = (tag ?? "").toLowerCase().split("-")[0] ?? "";
  return HELLO[lang] ?? "Hello";
}

export function Greeting() {
  const [word] = useState(() => {
    try {
      return helloFor(navigator.language);
    } catch {
      return "Hello";
    }
  });
  return (
    <h1 className={s.greeting} aria-label={`${word}.`}>
      <span className={s.greetingWord} aria-hidden="true">
        {word}
        <span className={s.greetingDot}>.</span>
      </span>
    </h1>
  );
}
