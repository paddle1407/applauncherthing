use std::io;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct PowerOption {
    pub name: String,
    pub command: Vec<String>,
}

pub fn all_power_options() -> Vec<PowerOption> {
    vec![
        PowerOption {
            name: "Lock Screen".to_string(),
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "loginctl lock-session || true".to_string(),
            ],
        },
        PowerOption {
            name: "Sleep / Suspend".to_string(),
            command: vec!["loginctl".to_string(), "suspend".to_string()],
        },
        PowerOption {
            name: "Reboot System".to_string(),
            command: vec!["loginctl".to_string(), "reboot".to_string()],
        },
        PowerOption {
            name: "Power Off".to_string(),
            command: vec!["loginctl".to_string(), "poweroff".to_string()],
        },
        PowerOption {
            name: "Logout".to_string(),
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "mmsg dispatch quit 2>/dev/null || loginctl terminate-session \"\" 2>/dev/null || loginctl terminate-user \"\" 2>/dev/null || true"
                    .to_string(),
            ],
        },
    ]
}

pub fn execute_power(option: &PowerOption) -> io::Result<()> {
    let mut parts = option.command.clone();
    if parts.is_empty() {
        return Ok(());
    }
    let prog = parts.remove(0);
    Command::new(prog).args(parts).spawn()?.wait().map(|_| ())
}
