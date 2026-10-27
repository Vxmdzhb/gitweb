//! The read-only JSON API: the same repositories, trees, files, log and
//! commits the HTML pages show, for a client that renders them itself (the
//! AstrX site's git module).
//!
//! | Path | Returns |
//! |------|---------|
//! | `/repos.json` | every repository: name, description, last commit time |
//! | `/<repo>/summary.json` | default branch, branches, tags, commit count, clone URL, latest commits |
//! | `/<repo>/tree.json?ref=&path=` | a directory's entries, and its README as text |
//! | `/<repo>/blob.json?ref=&path=` | one file as text (capped), or `binary: true` |
//! | `/<repo>/log.json?ref=&page=` | one page of the commit log |
//! | `/<repo>/commit.json?id=` | one commit's metadata and its diff (capped) |
//!
//! Every input goes through the same validators as the HTML views
//! ([`SafeRef`], [`SafePath`]), so the API reaches nothing the pages can't. It
//! never writes. Errors are JSON too: `{"error": "..."}` with the status the
//! HTML page would have had. Text is returned as given by git (lossily decoded
//! to UTF-8); the client escapes it.

use super::{page_param, param, Request, Resp, Server};
use crate::gitcmd::{
    branches, commit_count, commit_meta, commit_patch, default_branch, discover_repos, is_binary,
    list_tree, log, read_blob, resolve_commit, stat_object, tags, valid_path, GitError, Repo,
    SafePath, SafeRef,
};

/// The most of a file `blob.json` returns; the rest is reported as truncated.
const BLOB_TEXT_MAX: usize = 512 * 1024;
/// The most of a diff `commit.json` returns.
const DIFF_TEXT_MAX: usize = 256 * 1024;
/// The README `tree.json` includes, at most.
const README_TEXT_MAX: usize = 64 * 1024;

/// A JSON string literal for `s`.
pub(crate) fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // `<`, `>` and `&` too, so the body is safe even if something
            // mistakes it for HTML.
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn opt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "null".to_string(), |n| n.to_string())
}

/// Text from git output, cut to at most `max` bytes on a character boundary.
fn capped_text(data: &[u8], max: usize) -> (String, bool) {
    let truncated = data.len() > max;
    let mut text = String::from_utf8_lossy(&data[..data.len().min(max)]).into_owned();
    if truncated {
        // A cut can land inside a multi-byte character, which decodes to U+FFFD.
        while text.ends_with('\u{fffd}') {
            text.pop();
        }
    }
    (text, truncated)
}

fn commit_row_json(sha: &str, short: &str, author: &str, ts: i64, subject: &str) -> String {
    format!(
        "{{\"sha\":{},\"short\":{},\"author\":{},\"ts\":{},\"subject\":{}}}",
        json_str(sha),
        json_str(short),
        json_str(author),
        ts,
        json_str(subject)
    )
}

impl Server {
    /// A JSON reply with the standard safety headers.
    pub(super) fn json(&self, code: u16, body: String) -> Resp {
        self.bytes(
            code,
            body.into_bytes(),
            "application/json; charset=utf-8",
            None,
            None,
        )
    }

    /// A `GitError` as a JSON error body with the matching status.
    pub(super) fn json_error(&self, err: &GitError) -> Resp {
        let (code, message) = match err {
            GitError::BadRequest(m) => (400, m.clone()),
            GitError::NotFound(m) => (404, m.clone()),
            GitError::Failed(m) => (500, format!("git error: {m}")),
        };
        self.json(code, format!("{{\"error\":{}}}", json_str(&message)))
    }

    /// `/repos.json`.
    pub(super) fn api_repos(&self) -> Result<Resp, GitError> {
        let repos = discover_repos(&self.config.root)?;
        let rows: Vec<String> = repos
            .iter()
            .map(|r| {
                format!(
                    "{{\"name\":{},\"description\":{},\"bare\":{},\"last_commit_ts\":{}}}",
                    json_str(&r.name),
                    json_str(r.description.trim()),
                    r.bare,
                    opt_i64(r.last_commit_ts)
                )
            })
            .collect();
        Ok(self.json(200, format!("{{\"repos\":[{}]}}", rows.join(","))))
    }

    /// `/<repo>/summary.json`.
    pub(super) fn api_summary(&self, req: &Request<'_>, repo: &Repo) -> Result<Resp, GitError> {
        let branch = default_branch(repo)?;
        let reference = SafeRef::parse(&branch);
        let commits = match &reference {
            Some(r) => match log(repo, r, 0, self.config.summary_commits) {
                Ok(rows) => rows,
                Err(GitError::NotFound(_)) => Vec::new(),
                Err(e) => return Err(e),
            },
            None => Vec::new(),
        };
        let count = match &reference {
            Some(r) => commit_count(repo, r).unwrap_or(0),
            None => 0,
        };
        let names = |rows: Vec<crate::gitcmd::RefRow>| -> String {
            rows.iter()
                .map(|r| json_str(&r.name))
                .collect::<Vec<_>>()
                .join(",")
        };
        let clone_url = if self.config.enable_clone {
            self.clone_url(req, repo)
        } else {
            None
        };
        let rows: Vec<String> = commits
            .iter()
            .map(|c| commit_row_json(&c.sha, &c.short, &c.author, c.ts, &c.subject))
            .collect();
        Ok(self.json(
            200,
            format!(
                "{{\"name\":{},\"description\":{},\"default_branch\":{},\"branches\":[{}],\
                 \"tags\":[{}],\"commit_count\":{},\"clone_url\":{},\"last_commit_ts\":{},\
                 \"commits\":[{}]}}",
                json_str(&repo.name),
                json_str(repo.description.trim()),
                json_str(&branch),
                names(branches(repo)?),
                names(tags(repo)?),
                count,
                clone_url
                    .as_deref()
                    .map_or_else(|| "null".to_string(), json_str),
                opt_i64(repo.last_commit_ts),
                rows.join(",")
            ),
        ))
    }

    /// `/<repo>/tree.json?ref=&path=`.
    pub(super) fn api_tree(
        &self,
        repo: &Repo,
        params: &[(String, String)],
    ) -> Result<Resp, GitError> {
        let reference = self.resolve_ref(repo, params)?;
        let path = Self::require_path(params)?;
        let st = stat_object(repo, &reference, &path)
            .ok_or_else(|| GitError::NotFound("no such path".to_string()))?;
        if st.otype != "tree" && st.otype != "commit" {
            return Err(GitError::BadRequest("not a directory".to_string()));
        }
        let mut entries = list_tree(repo, &reference, &path)?;
        // Directories first, then files, each by name: how people browse.
        entries.sort_by(|a, b| {
            (a.otype != "tree")
                .cmp(&(b.otype != "tree"))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        let total = entries.len();
        let cap = self.config.tree_page_size.max(1);
        let rows: Vec<String> = entries
            .iter()
            .take(cap)
            .map(|e| {
                format!(
                    "{{\"name\":{},\"path\":{},\"type\":{},\"mode\":{},\"size\":{}}}",
                    json_str(&e.name),
                    json_str(&e.path),
                    json_str(&e.otype),
                    json_str(&e.mode),
                    e.size.map_or_else(|| "null".to_string(), |s| s.to_string())
                )
            })
            .collect();
        let readme = entries
            .iter()
            .find(|e| {
                e.otype == "blob"
                    && valid_path(&e.path)
                    && e.name.to_lowercase().starts_with("readme")
            })
            .and_then(|e| {
                let p = SafePath::parse(&e.path)?;
                let data = read_blob(repo, &reference, &p, README_TEXT_MAX + 1).ok()?;
                if is_binary(&data) {
                    return None;
                }
                let (text, truncated) = capped_text(&data, README_TEXT_MAX);
                Some(format!(
                    "{{\"name\":{},\"text\":{},\"truncated\":{}}}",
                    json_str(&e.name),
                    json_str(&text),
                    truncated
                ))
            });
        Ok(self.json(
            200,
            format!(
                "{{\"ref\":{},\"commit\":{},\"path\":{},\"total\":{},\"truncated\":{},\
                 \"entries\":[{}],\"readme\":{}}}",
                json_str(reference.as_str()),
                json_str(&resolve_commit(repo, &reference)),
                json_str(path.as_str()),
                total,
                total > cap,
                rows.join(","),
                readme.unwrap_or_else(|| "null".to_string())
            ),
        ))
    }

    /// `/<repo>/blob.json?ref=&path=`.
    pub(super) fn api_blob(
        &self,
        repo: &Repo,
        params: &[(String, String)],
    ) -> Result<Resp, GitError> {
        let reference = self.resolve_ref(repo, params)?;
        let path = Self::require_path(params)?;
        let st = stat_object(repo, &reference, &path)
            .ok_or_else(|| GitError::NotFound("no such path".to_string()))?;
        if st.otype != "blob" {
            return Err(GitError::BadRequest("not a file".to_string()));
        }
        let data = read_blob(repo, &reference, &path, BLOB_TEXT_MAX + 1)?;
        let binary = is_binary(&data);
        let (text, truncated) = if binary {
            (String::new(), false)
        } else {
            capped_text(&data, BLOB_TEXT_MAX)
        };
        Ok(self.json(
            200,
            format!(
                "{{\"ref\":{},\"path\":{},\"size\":{},\"binary\":{},\"truncated\":{},\"text\":{}}}",
                json_str(reference.as_str()),
                json_str(path.as_str()),
                st.size,
                binary,
                truncated,
                json_str(&text)
            ),
        ))
    }

    /// `/<repo>/log.json?ref=&page=`.
    pub(super) fn api_log(
        &self,
        repo: &Repo,
        params: &[(String, String)],
    ) -> Result<Resp, GitError> {
        let reference = self.resolve_ref(repo, params)?;
        let page = page_param(params);
        let size = self.config.page_size.max(1);
        // One extra row tells whether another page follows.
        let mut rows = log(repo, &reference, (page - 1).saturating_mul(size), size + 1)?;
        let has_more = rows.len() > size;
        rows.truncate(size);
        let commits: Vec<String> = rows
            .iter()
            .map(|c| commit_row_json(&c.sha, &c.short, &c.author, c.ts, &c.subject))
            .collect();
        Ok(self.json(
            200,
            format!(
                "{{\"ref\":{},\"page\":{},\"has_more\":{},\"commits\":[{}]}}",
                json_str(reference.as_str()),
                page,
                has_more,
                commits.join(",")
            ),
        ))
    }

    /// `/<repo>/commit.json?id=`.
    pub(super) fn api_commit(
        &self,
        repo: &Repo,
        params: &[(String, String)],
    ) -> Result<Resp, GitError> {
        let id = param(params, "id").trim();
        let rev =
            SafeRef::parse(id).ok_or_else(|| GitError::BadRequest("invalid id".to_string()))?;
        let meta = commit_meta(repo, &rev)?;
        let patch = commit_patch(repo, &rev)?;
        let (diff, truncated) = capped_text(patch.as_bytes(), DIFF_TEXT_MAX);
        let parents: Vec<String> = meta.parents.iter().map(|p| json_str(p)).collect();
        Ok(self.json(
            200,
            format!(
                "{{\"sha\":{},\"short\":{},\"author_name\":{},\"author_email\":{},\
                 \"author_date\":{},\"committer_name\":{},\"committer_date\":{},\
                 \"parents\":[{}],\"subject\":{},\"body\":{},\"signed\":{},\
                 \"signature_verified\":{},\"diff\":{},\"diff_truncated\":{}}}",
                json_str(&meta.sha),
                json_str(&meta.short),
                json_str(&meta.author_name),
                json_str(&meta.author_email),
                json_str(&meta.author_date),
                json_str(&meta.committer_name),
                json_str(&meta.committer_date),
                parents.join(","),
                json_str(&meta.subject),
                json_str(&meta.body),
                meta.signature_present(),
                meta.signature_verified(),
                json_str(&diff),
                truncated
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{capped_text, json_str};

    #[test]
    fn json_str_escapes_controls_quotes_and_markup() {
        assert_eq!(json_str("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(json_str("x\ny\t\u{1}"), r#""x\ny\t\u0001""#);
        assert_eq!(json_str("<script>&"), r#""\u003cscript\u003e\u0026""#);
        assert_eq!(json_str("\u{2028}"), r#""\u2028""#);
    }

    #[test]
    fn capped_text_cuts_on_a_character_boundary() {
        let (t, cut) = capped_text("héllo".as_bytes(), 2);
        assert_eq!((t.as_str(), cut), ("h", true));
        let (t, cut) = capped_text(b"abc", 10);
        assert_eq!((t.as_str(), cut), ("abc", false));
    }
}
