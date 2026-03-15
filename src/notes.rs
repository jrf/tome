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
                    set noteIds to id of every note of f
                    set noteNames to name of every note of f
                    repeat with j from 1 to count of noteIds
                        set end of output to folderName & "|||" & (item j of noteIds) & "|||" & (item j of noteNames)
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
                    folder: parts[0].trim().to_string(),
                    id: parts[1].trim().to_string(),
                    name: parts[2].trim().to_string(),
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

    let name = parts[2].trim().to_string();
    let raw_body = parts[3].trim();

    // Apple Notes' plaintext includes the title as the first line.
    // Strip it so editing doesn't duplicate the title on save.
    let body = raw_body
        .strip_prefix(&name)
        .map(|s| s.trim_start_matches('\n'))
        .unwrap_or(raw_body)
        .to_string();

    Ok(Note {
        id: parts[1].trim().to_string(),
        name,
        folder: parts[0].trim().to_string(),
        body,
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
                    set matched to (every note of f whose name contains "{escaped}")
                    set noteIds to id of matched
                    set noteNames to name of matched
                    repeat with j from 1 to count of noteIds
                        set end of output to folderName & "|||" & (item j of noteIds) & "|||" & (item j of noteNames)
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
                    folder: parts[0].trim().to_string(),
                    id: parts[1].trim().to_string(),
                    name: parts[2].trim().to_string(),
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

pub fn update_note(name: &str, new_title: &str, new_body: &str) -> Result<()> {
    let escaped_name = escape_applescript(name);
    let escaped_title = escape_applescript(new_title);
    let html_body = escape_applescript(&new_body.replace('\n', "<br>"));

    let script = format!(
        r#"tell application "Notes"
            set matchedNotes to (every note whose name contains "{escaped_name}")
            if (count of matchedNotes) is 0 then
                error "No note found matching: {escaped_name}"
            end if
            set n to item 1 of matchedNotes
            set body of n to "<h1>{escaped_title}</h1><br>{html_body}"
        end tell"#
    );

    run_applescript(&script)?;
    Ok(())
}

pub fn move_note(name: &str, to_folder: &str) -> Result<()> {
    let escaped_name = escape_applescript(name);
    let escaped_folder = escape_applescript(to_folder);

    let script = format!(
        r#"tell application "Notes"
            set matchedNotes to (every note whose name contains "{escaped_name}")
            if (count of matchedNotes) is 0 then
                error "No note found matching: {escaped_name}"
            end if
            move item 1 of matchedNotes to folder "{escaped_folder}"
        end tell"#
    );

    run_applescript(&script)?;
    Ok(())
}

pub fn delete_note(name: &str) -> Result<()> {
    let escaped_name = escape_applescript(name);

    let script = format!(
        r#"tell application "Notes"
            set matchedNotes to (every note whose name contains "{escaped_name}")
            if (count of matchedNotes) is 0 then
                error "No note found matching: {escaped_name}"
            end if
            delete item 1 of matchedNotes
        end tell"#
    );

    run_applescript(&script)?;
    Ok(())
}

pub fn list_folders() -> Result<Vec<String>> {
    let script = r#"tell application "Notes"
            set output to {}
            repeat with f in every folder
                if name of f is not "Recently Deleted" then
                    set end of output to name of f
                end if
            end repeat
            set AppleScript's text item delimiters to "\n"
            return output as text
        end tell"#;

    let output = run_applescript(script)?;
    if output.is_empty() {
        return Ok(vec![]);
    }

    Ok(output.lines().map(|l| l.trim().to_string()).collect())
}

pub fn create_folder(name: &str) -> Result<()> {
    let escaped = escape_applescript(name);
    let script = format!(
        r#"tell application "Notes"
            make new folder with properties {{name:"{escaped}"}}
        end tell"#
    );
    run_applescript(&script)?;
    Ok(())
}

pub fn rename_folder(old_name: &str, new_name: &str) -> Result<()> {
    let escaped_old = escape_applescript(old_name);
    let escaped_new = escape_applescript(new_name);
    let script = format!(
        r#"tell application "Notes"
            set name of folder "{escaped_old}" to "{escaped_new}"
        end tell"#
    );
    run_applescript(&script)?;
    Ok(())
}

pub fn delete_folder(name: &str) -> Result<()> {
    let escaped = escape_applescript(name);
    let script = format!(
        r#"tell application "Notes"
            delete folder "{escaped}"
        end tell"#
    );
    run_applescript(&script)?;
    Ok(())
}
