use anyhow::Result;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;

pub fn connect() -> Result<Docker> {
    Ok(Docker::connect_with_unix_defaults()?)
}

pub async fn get_container_id(docker: &Docker, service: &str) -> Option<String> {
    let filters = HashMap::from([(
        "label".to_string(),
        vec![format!("com.docker.compose.service={service}")],
    )]);
    let containers = docker
        .list_containers(Some(bollard::container::ListContainersOptions {
            filters,
            ..Default::default()
        }))
        .await
        .ok()?;
    containers.first()?.id.clone()
}

/// Fetch container logs since a given timestamp (RFC3339).
pub async fn get_container_logs(
    docker: &Docker,
    container_id: &str,
    since: &str,
) -> Result<String> {
    use bollard::container::LogsOptions;

    // Parse RFC3339 to unix timestamp for the Docker API
    let timestamp = chrono::DateTime::parse_from_rfc3339(since)
        .map(|dt| dt.timestamp())
        .unwrap_or(0);

    let opts = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        since: timestamp,
        timestamps: true,
        ..Default::default()
    };

    let mut stream = docker.logs(container_id, Some(opts));
    let mut output = String::new();
    while let Some(Ok(chunk)) = stream.next().await {
        match chunk {
            bollard::container::LogOutput::StdOut { message }
            | bollard::container::LogOutput::StdErr { message } => {
                output.push_str(&String::from_utf8_lossy(&message));
            }
            _ => {}
        }
    }
    Ok(output)
}

pub async fn exec_in_container(
    docker: &Docker,
    container_id: &str,
    cmd: Vec<&str>,
) -> Result<String> {
    let exec = docker
        .create_exec(
            container_id,
            bollard::exec::CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            },
        )
        .await?;

    let mut output = String::new();
    if let bollard::exec::StartExecResults::Attached {
        output: mut stream, ..
    } = docker.start_exec(&exec.id, None).await?
    {
        while let Some(Ok(chunk)) = stream.next().await {
            match chunk {
                bollard::container::LogOutput::StdOut { message }
                | bollard::container::LogOutput::StdErr { message } => {
                    output.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            }
        }
    }
    Ok(output)
}
