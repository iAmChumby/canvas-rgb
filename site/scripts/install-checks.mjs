// The two checks that tie the install page's hash to the hash the build used (issues #83, #91).
//
// They live in their own module so a test can run them on a page without building the site;
// check-install.mjs prints the lines they return. The messages are the ones the script printed
// before these checks moved here, so its output is unchanged.

/**
 * Run the two hash checks on a page.
 *
 * `setup` is site/.setup-sha.json: `{ version, file, sha256 }`. Returns `{ ok, fail }`, the lines
 * the script prints. A page fails when it does not hold the hash the build used, character for
 * character: one wrong character is a different page, and the release it advertises is not the
 * one this build was made for.
 */
export function checkInstallHashes(html, setup) {
  const ok = [];
  const fail = [];
  const check = (passed, message) => (passed ? ok : fail).push(message);

  // The hash is a SHA-256: 64 lowercase hex characters.
  check(
    /^[0-9a-f]{64}$/.test(setup.sha256 ?? ""),
    `the hash the build used is 64 hex characters (${setup.sha256})`,
  );

  // The page shows it, in full.
  check(
    html.includes(setup.sha256),
    `the page shows the hash ${setup.sha256}`,
  );

  return { ok, fail };
}

/**
 * Whether the release carries the hash the build used, as the line check-install.mjs prints.
 *
 * `release` is the hash read from the GitHub release (release-sha.mjs). Returns
 * `{ passed, message }`. One wrong character is a different file (#91).
 */
export function checkReleaseHash(release, setup) {
  return {
    passed: release === setup.sha256,
    message: `the release v${setup.version} carries the same hash (${release})`,
  };
}
