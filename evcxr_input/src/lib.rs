/// Requests a string from the user with the specified prompt.
pub fn get_string(prompt: &str) -> String {
    get_input(prompt, false).unwrap_or_default()
}

/// Requests a string from the user with the specified prompt, treating the input as a password.
pub fn get_password(prompt: &str) -> String {
    get_input(prompt, true).unwrap_or_default()
}

fn get_input(prompt: &str, is_password: bool) -> Option<String> {
    if is_password {
        println!("{}:{}", GET_CMD_PASSWORD, prompt);
    } else {
        println!("{}:{}", GET_CMD, prompt);
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line.trim().to_owned())
}

// The following constants are here so that they can be shared between this crate and Evcxr. They're
// not really intended to be used.

#[doc(hidden)]
pub const GET_CMD: &str = "EVCXR_INPUT_REQUEST";

#[doc(hidden)]
pub const GET_CMD_PASSWORD: &str = "EVCXR_INPUT_REQUEST_PASSWORD";
