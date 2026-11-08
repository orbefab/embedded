# @orbe/brand

The Orbefab brand as this repo's site uses it: the mark, the favicons, the logo, the wordmark face, the colour tokens and the wordmark component.

`website/` holds files copied unchanged from [orbefab/website](https://github.com/orbefab/website), at their paths there, from the commit in `website.commit`. `website.sha256` lists each one with its SHA-256. Nothing here is edited by hand. A change to the brand lands in the website repo first, and is then copied here.

```sh
scripts/brand.sh check                    # every file is listed and has its listed hash
scripts/brand.sh check ../website         # and each one has its bytes at that commit there
scripts/brand.sh copy ../website          # copy every listed file again from the checkout's HEAD
```

`copy` reads the commit with git, so a checkout's uncommitted changes are never copied, and the checkout is never written. The site build runs `check` and refuses a copy that differs from its list.

| Export | File |
| --- | --- |
| `@orbe/brand/styles.css` | The colour, radius and font tokens for Tailwind 4, and the Syne `@font-face`. It loads Space Grotesk and Space Mono from Google Fonts, as the website does. |
| `@orbe/brand/mark` | `Ring`, the mark, and `Wordmark`, the ring set as the O of Orbefab in Syne. |
| `@orbe/brand/colors` | The ink, paper and accent colours as hex, for places that cannot read CSS variables. |

`website/public/` is the site's public folder, so the favicons, the logo and the font are served beside its pages.

## Licence

The files here are under MIT OR Apache-2.0, as the rest of this repo, except the Syne font. `website/public/fonts/Syne-VariableFont_wght.ttf` is Syne, Copyright The Syne Project Authors, under the SIL Open Font License 1.1, which is in [`OFL.txt`](OFL.txt). The site serves that licence beside the font.

## Trademark

Orbefab, the Orbefab name and the Orbefab logo, the ring with its dot, are trademarks. The licences above cover the files and grant no right to the trademarks. You may use the name to refer to Orbefab and to say truthfully that something works with it. Do not use the name or the logo for another product or service, or in a way that suggests Orbefab made, sponsors or endorses it, without written permission. A modified copy of the site or of `orbe` that you publish must not carry the logo or call itself Orbefab.
