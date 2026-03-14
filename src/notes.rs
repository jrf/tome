use anyhow::{Context, Result, bail};
use std::process::Command;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub body: String,
}

fn run_applescript(script: &str) -> Result<String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .context("Failed to run osascript")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("AppleScript error: {}", err.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn list_notes(folder: Option<&str>) -> Result<Vec<Note>> {
    let folder_loop = match folder {
        Some(f) => format!("{{folder \"{}\" of default account}}", escape_applescript(f)),
        None => "every folder".to_string(),
    };

    let script = format!(
        r#"tell application "Notes"
            set output to {{}}
            repeat with f in {folder_loop}
                set folderName to name of f
                if folderName is not "Recently Deleted" then
                    repeat with n in every note of f
                        set end of output to folderName & "|||" & (id of n) & "|||" & (name of n)
                    end repeat
                end if
            end repeat
            set AppleScript's text item delimiters to "\n"
            return output as text
        end tell"#
    );

    let output = run_applescript(&script)?;
    if output.is_empty() {
        return Ok(vec![]);
    }

    Ok(output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, "|||").collect();
            if parts.len() >= 3 {
                Some(Note {
                    id: parts[1].trim().to_string(),
                    name: parts[2].trim().to_string(),
                    folder: parts[0].trim().to_string(),
                    body: String::new(),
                })
            } else {
                None
            }
        })
        .collect())
}

pub fn get_note(name: &str) -> Result<Note> {
    let escaped = escape_applescript(name);
    let script = format!(
        r#"tell application "Notes"
            repeat with f in every folder
                set folderName to name of f
                if folderName is not "Recently Deleted" then
                    set matched to (every note of f whose name contains "{escaped}")
                    if (count of matched) > 0 then
                        set n to item 1 of matched
                        return folderName & "|||" & (id of n) & "|||" & (name of n) & "|||" & (plaintext of n)
                    end if
                end if
            end repeat
            error "No note found matching: {escaped}"
        end tell"#
    );

    let output = run_applescript(&script)?;
    let parts: Vec<&str> = output.splitn(4, "|||").collect();
    if parts.len() < 4 {
        bail!("Could not parse note response");
    }

    Ok(Note {
        id: parts[1].trim().to_string(),
        name: parts[2].trim().to_string(),
        folder: parts[0].trim().to_string(),
        body: parts[3].trim().to_string(),
    })
}

pub fn search_notes(query: &str) -> Result<Vec<Note>> {
    let escaped = escape_applescript(query);
    let script = format!(
        r#"tell application "Notes"
            set output to {{}}
            repeat with f in every folder
                set folderName to name of f
                if folderName is not "Recently Deleted" then
                    repeat with n in (every note of f whose plaintext contains "{escaped}")
                        set end of output to folderName & "|||" & (id of n) & "|||" & (name of n)
                    end repeat
                end if
            end repeat
            set AppleScript's text item delimiters to "\n"
            return output as text
        end tell"#
    );

    let output = run_applescript(&script)?;
    if output.is_empty() {
        return Ok(vec![]);
    }

    Ok(output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, "|||").collect();
            if parts.len() >= 3 {
                Some(Note {
                    id: parts[1].trim().to_string(),
                    name: parts[2].trim().to_string(),
                    folder: parts[0].trim().to_string(),
                    body: String::new(),
                })
            } else {
                None
            }
        })
        .collect())
}

pub fn create_note(title: &str, body: &str, folder: Option<&str>) -> Result<()> {
    let escaped_title = escape_applescript(title);
    let html_body = escape_applescript(&body.replace('\n', "<br>"));
    let folder_target = match folder {
        Some(f) => format!("in folder \"{}\"", escape_applescript(f)),
        None => String::new(),
    };

    let script = format!(
        r#"tell application "Notes"
            make new note {folder_target} with properties {{name:"{escaped_title}", body:"{html_body}"}}
        end tell"#
    );

    run_applescript(&script)?;
    Ok(())
}

pub fn update_note_body(name: &str, new_body: &str) -> Result<()> {
    let escaped_name = escape_applescript(name);
    let html_body = escape_applescript(&new_body.replace('\n', "<br>"));

    let script = format!(
        r#"tell application "Notes"
            set matchedNotes to (every note whose name contains "{escaped_name}")
            if (count of matchedNotes) is 0 then
                error "No note found matching: {escaped_name}"
            end if
            set n to item 1 of matchedNotes
            set body of n to "<h1>{escaped_name}</h1><br>{html_body}"
        end tell"#
    );

    run_applescript(&script)?;
    Ok(())
}

pub fn list_folders() -> Result<Vec<String>> {
    let script = r#"tell application "Notes"
            set folderNames to name of every folder
            set AppleScript's text item delimiters to "\n"
            return folderNames as text
        end tell"#;

    let output = run_applescript(script)?;
    if output.is_empty() {
        return Ok(vec![]);
    }

    Ok(output.lines().map(|l| l.trim().to_string()).collect())
}
