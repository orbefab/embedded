//! The viewer page `show` serves: the one built into this `orbe`, or a built page's folder that
//! `ORBE_VIEWER_DIR` names.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use orbe_session::{Code, Error};

include!(concat!(env!("OUT_DIR"), "/page.rs"));

/// Names a built page's folder to serve in place of the built-in page.
pub(super) const FOLDER: &str = "ORBE_VIEWER_DIR";

pub(super) enum Page {
    Built(&'static [(&'static str, &'static [u8])]),
    Folder(PathBuf),
}

impl Page {
    /// The folder `ORBE_VIEWER_DIR` names, as an absolute path, else the built-in page.
    pub(super) fn find(folder: Option<&Path>) -> Result<Page, Error> {
        Page::choose(PAGE, folder)
    }

    fn choose(
        built: &'static [(&'static str, &'static [u8])],
        folder: Option<&Path>,
    ) -> Result<Page, Error> {
        match folder {
            Some(dir) if dir.join("index.html").is_file() => Ok(Page::Folder(dir.to_path_buf())),
            Some(dir) => Err(Error::new(
                Code::IoError,
                format!("{FOLDER}={} holds no index.html", dir.display()),
                format!("build the page into that folder, or unset {FOLDER}"),
            )),
            None if built.is_empty() => Err(Error::new(
                Code::IoError,
                "this orbe was built without the viewer page",
                "in the orbe repo, run `pnpm install --frozen-lockfile` and `pnpm --filter @orbe/viewer-page build`, then build orbe again with `cargo build`",
            )),
            None => Ok(Page::Built(built)),
        }
    }

    /// The file at a URL path such as `/` or `/assets/index.js`.
    pub(super) fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        let name = match path.trim_start_matches('/') {
            "" => "index.html",
            name => name,
        };
        match self {
            Page::Built(files) => files
                .iter()
                .find(|(file, _)| *file == name)
                .map(|(_, bytes)| Cow::Borrowed(*bytes)),
            Page::Folder(dir) => {
                let inside = Path::new(name)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)));
                inside
                    .then(|| std::fs::read(dir.join(name)).ok())
                    .flatten()
                    .map(Cow::Owned)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_orbe_built_without_the_page_says_how_to_build_it() {
        let Err(error) = Page::choose(&[], None) else {
            panic!("a page with no files");
        };
        assert_eq!(error.code, Code::IoError);
        assert_eq!(error.message, "this orbe was built without the viewer page");
        assert_eq!(
            error.hint,
            "in the orbe repo, run `pnpm install --frozen-lockfile` and `pnpm --filter @orbe/viewer-page build`, then build orbe again with `cargo build`"
        );
    }

    #[test]
    fn a_folder_serves_only_files_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let built = dir.path().join("dist");
        std::fs::create_dir(&built).unwrap();
        std::fs::write(built.join("index.html"), "page").unwrap();
        std::fs::write(dir.path().join("secret"), "secret").unwrap();
        let page = Page::choose(&[], Some(&built)).unwrap();
        assert_eq!(page.get("/").as_deref(), Some(&b"page"[..]));
        assert!(page.get("/../secret").is_none());
        assert!(page.get("/missing.js").is_none());
    }
}
