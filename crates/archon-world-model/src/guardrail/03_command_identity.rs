fn git_subcommand(tokens: &[String]) -> Option<&str> {
    let mut index = 1;
    while let Some(token) = tokens.get(index) {
        if !token.starts_with('-') {
            return Some(token);
        }
        index += 1;
        if matches!(token.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
            index += 1;
        }
    }
    None
}

fn is_verification_command(tokens: &[String]) -> bool {
    let Some(executable) = tokens.first().map(|token| executable_name(token)) else {
        return false;
    };
    let operation = tokens.get(1).map(String::as_str);
    let package_operation = package_manager_operation(tokens);
    matches!(
        (executable.as_str(), operation),
        ("cargo", Some("test" | "check" | "build" | "clippy" | "fmt")) | ("pytest", _)
    ) || matches!(
        (
            executable.as_str(),
            package_operation,
            package_manager_next_argument(tokens)
        ),
        ("npm" | "pnpm" | "yarn" | "bun", Some("test"), _)
            | (
                "npm" | "pnpm" | "yarn" | "bun",
                Some("run"),
                Some("test" | "build" | "lint" | "typecheck")
            )
    )
}

fn is_external_command(tokens: &[String]) -> bool {
    let Some(executable) = tokens.first().map(|token| executable_name(token)) else {
        return false;
    };
    (executable == "git" && git_subcommand(tokens) == Some("push"))
        || matches!(executable.as_str(), "deploy" | "publish")
        || matches!(
            (executable.as_str(), package_manager_operation(tokens)),
            ("npm" | "pnpm" | "yarn" | "bun", Some("publish" | "deploy"))
        )
}

fn package_manager_operation(tokens: &[String]) -> Option<&str> {
    let executable = tokens.first().map(|token| executable_name(token))?;
    if !matches!(executable.as_str(), "npm" | "pnpm" | "yarn" | "bun") {
        return tokens.get(1).map(String::as_str);
    }
    tokens
        .get(package_manager_operation_index(tokens)?)
        .map(String::as_str)
}

fn package_manager_next_argument(tokens: &[String]) -> Option<&str> {
    tokens
        .get(package_manager_operation_index(tokens)? + 1)
        .map(String::as_str)
}

fn package_manager_operation_index(tokens: &[String]) -> Option<usize> {
    let mut index = 1;
    while let Some(option) = tokens.get(index) {
        if option == "--" {
            return tokens.get(index + 1).map(|_| index + 1);
        }
        if !option.starts_with('-') {
            return Some(index);
        }
        if package_manager_option_requires_value(option) {
            tokens.get(index + 1)?;
            index += 2;
        } else if is_package_manager_flag(option) || option.contains('=') {
            index += 1;
        } else {
            return None;
        }
    }
    None
}

fn package_manager_option_requires_value(option: &str) -> bool {
    matches!(
        option,
        "--prefix"
            | "--cache"
            | "--registry"
            | "--userconfig"
            | "--workspace"
            | "--loglevel"
            | "--omit"
            | "--include"
            | "--tag"
    )
}

fn is_package_manager_flag(option: &str) -> bool {
    matches!(
        option,
        "--silent"
            | "-s"
            | "--json"
            | "--yes"
            | "-y"
            | "--force"
            | "-f"
            | "--ignore-scripts"
            | "--no-audit"
            | "--no-fund"
            | "--offline"
            | "--prefer-offline"
            | "--prefer-online"
            | "--workspaces"
            | "--include-workspace-root"
    )
}

fn is_destructive_command(tokens: &[String]) -> bool {
    tokens
        .first()
        .is_some_and(|token| executable_name(token) == "rm")
        || has_executable_sql_mutation(tokens)
}

fn has_executable_sql_mutation(tokens: &[String]) -> bool {
    let Some((database_client, arguments)) = tokens.split_first() else {
        return false;
    };
    is_database_client(database_client)
        && sql_command_argument(arguments).is_some_and(has_sql_mutation)
}

fn is_database_client(command: &str) -> bool {
    matches!(
        executable_name(command).as_str(),
        "psql" | "sqlite3" | "mysql" | "mariadb" | "sqlcmd"
    )
}

fn sql_command_argument(arguments: &[String]) -> Option<&str> {
    arguments
        .windows(2)
        .find(|pair| matches!(pair[0].as_str(), "-c" | "--command" | "-e" | "-Q"))
        .map(|pair| pair[1].as_str())
}

fn has_sql_mutation(sql: &str) -> bool {
    let mut word = String::new();
    let bytes = sql.as_bytes();
    let mut in_literal = false;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\'' {
            if in_literal && bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                continue;
            }
            in_literal = !in_literal;
        } else if !in_literal && byte == b'$' {
            if let Some((delimiter, content_start)) = sql_dollar_quote_delimiter(sql, index) {
                let remaining = &sql[content_start..];
                let Some(closing) = remaining.find(delimiter) else {
                    return false;
                };
                if sql_word_is_mutation(&word) {
                    return true;
                }
                word.clear();
                index = content_start + closing + delimiter.len();
                continue;
            }
        } else if !in_literal && byte.is_ascii_alphabetic() {
            word.push((byte as char).to_ascii_uppercase());
        } else if !in_literal && sql_word_is_mutation(&word) {
            return true;
        } else if !in_literal {
            word.clear();
        }
        index += 1;
    }

    !in_literal && sql_word_is_mutation(&word)
}

fn sql_dollar_quote_delimiter(sql: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = sql.as_bytes();
    let mut index = start + 1;
    if bytes.get(index) == Some(&b'$') {
        return Some((&sql[start..=index], index + 1));
    }
    if !matches!(bytes.get(index), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')) {
        return None;
    }
    index += 1;
    while matches!(bytes.get(index), Some(byte) if byte.is_ascii_alphanumeric() || *byte == b'_') {
        index += 1;
    }
    (bytes.get(index) == Some(&b'$')).then(|| (&sql[start..=index], index + 1))
}

fn sql_word_is_mutation(word: &str) -> bool {
    matches!(word, "DROP" | "DELETE")
}
