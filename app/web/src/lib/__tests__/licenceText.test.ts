// Reflowing the generated notices for the Credits disclosures (#584).
// The samples are taken from the generated notices: the ~72-column, indented
// text is the input, and each paragraph is expected to read as one line that
// CSS can wrap at any width.

import { describe, expect, it } from "vitest";
import { reflowLicenceText } from "@/lib/licenceText";

const MIT = `    MIT License

    Copyright (c) Microsoft Corporation.

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.
`;

const APACHE = ` 
                             Apache License
                       Version 2.0, January 2004
                    http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

   END OF TERMS AND CONDITIONS
`;

describe("reflowLicenceText", () => {
  it("dedents an MIT notice and joins each hard-wrapped paragraph into one line", () => {
    expect(reflowLicenceText(MIT)).toEqual([
      "MIT License",
      "Copyright (c) Microsoft Corporation.",
      'Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:',
      "The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.",
    ]);
  });

  it("keeps the notice separator and the copyright line on their own lines", () => {
    expect(reflowLicenceText(`[1]\n${MIT}`).slice(0, 3)).toEqual([
      "[1]",
      "MIT License",
      "Copyright (c) Microsoft Corporation.",
    ]);
  });

  it("breaks before a copyright line even without a blank line", () => {
    const sample = `    MIT License
    Copyright (c) Acme.
    Permission is hereby granted, free of charge, to any person obtaining a copy`;
    expect(reflowLicenceText(sample)).toEqual([
      "MIT License",
      "Copyright (c) Acme.",
      "Permission is hereby granted, free of charge, to any person obtaining a copy",
    ]);
  });

  it("keeps the Apache title, version and URL as separate lines", () => {
    expect(reflowLicenceText(APACHE).slice(0, 5)).toEqual([
      "Apache License",
      "Version 2.0, January 2004",
      "http://www.apache.org/licenses/",
      "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",
      "1. Definitions.",
    ]);
  });

  it("joins Apache numbered sections and lettered items into readable paragraphs", () => {
    const paragraphs = reflowLicenceText(APACHE);
    expect(paragraphs).toContain(
      '"License" shall mean the terms and conditions for use, reproduction, and distribution as defined by Sections 1 through 9 of this document.',
    );
    expect(paragraphs).toContain(
      "2. Grant of Copyright License. Subject to the terms and conditions of this License, each Contributor hereby grants to You a perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable copyright license to reproduce, prepare Derivative Works of, publicly display, publicly perform, sublicense, and distribute the Work and such Derivative Works in Source or Object form.",
    );
    expect(paragraphs).toContain(
      "4. Redistribution. You may reproduce and distribute copies of the Work or Derivative Works thereof in any medium, with or without modifications, and in Source or Object form, provided that You meet the following conditions:",
    );
    expect(paragraphs).toContain(
      "(a) You must give any other recipients of the Work or Derivative Works a copy of this License; and",
    );
    expect(paragraphs).toContain(
      "(b) You must cause any modified files to carry prominent notices stating that You changed the files; and",
    );
    expect(paragraphs.at(-1)).toBe("END OF TERMS AND CONDITIONS");
  });

  it("leaves no line breaks or leading indentation in any paragraph", () => {
    for (const sample of [MIT, APACHE, `[1]\n${MIT}`]) {
      for (const paragraph of reflowLicenceText(sample)) {
        expect(paragraph).not.toContain("\n");
        expect(paragraph).toBe(paragraph.trim());
      }
    }
  });

  it("is pure: empty notices give no paragraphs and repeat calls agree", () => {
    expect(reflowLicenceText("")).toEqual([]);
    expect(reflowLicenceText("  \n\n \n")).toEqual([]);
    expect(reflowLicenceText(MIT)).toEqual(reflowLicenceText(MIT));
    expect(MIT.startsWith("    MIT License")).toBe(true);
  });
});
