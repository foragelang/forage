# Homebrew tap (staging directory)

This directory holds the canonical Forage Homebrew formula. It is **not** the
actual tap — Homebrew taps must live in a repository named
`<owner>/homebrew-<tap-name>`. For Forage that's `foragelang/homebrew-tap`.

## Layout

```
homebrew-tap/
├── README.md          (this file)
└── Formula/
    └── forage.rb      (canonical forage formula)
```

## Publishing the tap for the first time

1. Create the repository `foragelang/homebrew-tap` on GitHub (empty, public).
2. Clone it locally and copy `Formula/forage.rb` into it:

   ```sh
   git clone git@github.com:foragelang/homebrew-tap.git
   cp Formula/forage.rb homebrew-tap/Formula/forage.rb
   cd homebrew-tap
   git add Formula/forage.rb
   git commit -m "Add forage formula"
   git push
   ```

3. Verify the install path works:

   ```sh
   brew install foragelang/forage/forage
   forage --help
   ```

The user-facing tap reference uses `foragelang/forage` (`brew tap <owner>/<name>`,
where `homebrew-` is implicit). The repository name on GitHub is
`foragelang/homebrew-tap`.

## Automated updates

Once `foragelang/homebrew-tap` exists, the release workflow can keep the
formula fresh on every tag push. To enable:

1. In `foragelang/forage`, create a fine-grained PAT with `contents: write`
   on `foragelang/homebrew-tap`.
2. Add the PAT as the `HOMEBREW_TAP_TOKEN` secret.
3. Set the repository variable `ENABLE_HOMEBREW_TAP_UPDATE=1`.

Each tag push then triggers the `update-homebrew-tap` job in
`.github/workflows/release.yml`, which rewrites `Formula/forage.rb` in the tap
repo and pushes the change.

Until that's wired up, after each release update `Formula/forage.rb` in this
directory by hand:

- `url` → the new release tarball
- `sha256` → the new tarball's sha256 (from `*.sha256` next to the tarball)

…and copy the file into the tap repo.

## Why this directory exists

Keeping the formula in the main repo means:

- it travels with the codebase (PRs touching releases touch the formula too);
- new contributors see how Homebrew distribution works without hunting for a
  second repo;
- the release workflow has a known source-of-truth for what the formula
  should look like.

The tap repo holds an exact copy. The main repo is canonical.
