use crate::config::{GitopsConfig, Settings};
use crate::models::Workload;
use git2::{
    Commit, Cred, ErrorCode, IndexAddOption, PushOptions, RemoteCallbacks, Repository, Signature,
};
use serde_yaml_ng::value::TaggedValue;
use serde_yaml_ng::Value as YamlValue;
use walkdir::WalkDir;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use crate::notifications::ntfy::notify_commit;

fn parse_k8s_yaml<T: serde::de::DeserializeOwned>(contents: &str) -> Result<T, String> {
    let yaml: YamlValue =
        serde_yaml_ng::from_str(contents).map_err(|e| format!("YAML parse error: {}", e))?;
    let fixed = fix_octal_strings(&yaml);
    let json = serde_json::to_value(&fixed)
        .map_err(|e| format!("YAML→JSON conversion error: {}", e))?;
    serde_json::from_value(json).map_err(|e| format!("K8s deserialization error: {}", e))
}

fn fix_octal_strings(v: &YamlValue) -> YamlValue {
    match v {
        YamlValue::Mapping(m) => {
            let mapped = m.iter().map(|(k, val)| (k.clone(), fix_octal_strings(val)));
            YamlValue::Mapping(mapped.collect())
        }
        YamlValue::Sequence(s) => {
            YamlValue::Sequence(s.iter().map(fix_octal_strings).collect())
        }
        YamlValue::Number(_) | YamlValue::Bool(_) | YamlValue::Null => v.clone(),
        YamlValue::String(s) => {
            if let Some(n) = s.strip_prefix("0o") {
                if let Ok(val) = i64::from_str_radix(n, 8) {
                    return YamlValue::Number(serde_yaml_ng::Number::from(val));
                }
            }
            if s.len() >= 2
                && s.starts_with('0')
                && s[1..].chars().all(|c| c.is_ascii_digit())
            {
                if let Ok(val) = i64::from_str_radix(s, 8) {
                    return YamlValue::Number(serde_yaml_ng::Number::from(val));
                }
            }
            v.clone()
        }
        YamlValue::Tagged(t) => {
            YamlValue::Tagged(Box::new(TaggedValue {
                tag: t.tag.clone(),
                value: fix_octal_strings(&t.value),
            }))
        }
    }
}

fn load_settings() -> Result<Vec<GitopsConfig>, String> {
    let settings = Settings::new().unwrap_or_else(|err| {
        log::error!("Failed to load settings: {}", err);
        panic!("Failed to load settings: {}", err);
    });
    if let Some(gitops_config) = settings.gitops {
        Ok(gitops_config.clone())
    } else {
        Err("No Gitops Config Found".to_string())
    }
}

fn delete_local_repo() -> Result<(), std::io::Error> {
    let local_path = Path::new("/tmp/repos/");
    if local_path.exists() {
        std::fs::remove_dir_all(local_path)?;
    }
    Ok(())
}

fn clone_or_open_repo(
    repo_url: &str,
    repo_path: &Path,
    access_token: &str,
) -> Result<Repository, git2::Error> {
    match Repository::open(repo_path) {
        Ok(repo) => Ok(repo),
        Err(e) if e.code() == ErrorCode::NotFound => {
            let mut cb = RemoteCallbacks::new();
            log::info!("Setting credentials");
            cb.credentials(move |_url, _username, _allowed_types| {
                Cred::userpass_plaintext("x-access-token", access_token)
            });
            log::info!("Setting credentials Done");

            let mut fo = git2::FetchOptions::new();
            fo.remote_callbacks(cb);

            let mut builder = git2::build::RepoBuilder::new();
            builder.fetch_options(fo);
            log::info!("Building repo");
            log::info!("Repo URL: {}", repo_url);
            log::info!("Repo Path: {:?}", repo_path);
            builder.clone(repo_url, repo_path)
        }
        Err(e) => Err(e),
    }
}

fn edit_files(local_path: &Path, workload: &Workload) {
    let name = &workload.name;
    let search_path = if let Some(git_directory) = &workload.git_directory {
        if git_directory.is_empty() {
            log::info!("No git directory specified for workload: {}", name);
            local_path.join(name)
        } else {
            log::info!("git directory: {:?}", git_directory);
            log::info!("Full Path: {:?}/{:?}", local_path, git_directory);
            local_path.join(git_directory)
        }
    } else {
        log::info!("No git directory specified for workload: {}", name);
        local_path.join(name)
    };
    let image = workload.image.clone();
    let base_image = image.split(":").collect::<Vec<&str>>()[0];
    let new_image = format!("{}:{}", base_image, workload.latest_version);
    log::info!("Base image: {}", &base_image);
    log::info!("New image: {}", &new_image);
    for entry in WalkDir::new(search_path).into_iter().filter_map(|e| e.ok()) {
        log::info!("Entry: {:?}", entry.path());
        if entry.path().extension().unwrap_or_default() == "yaml" {
            log::info!("YAML file found: {:?}", entry.path());
            let mut file = File::open(entry.path()).unwrap();
            let mut contents = String::new();
            file.read_to_string(&mut contents).unwrap();
            let mut image_updated = false;
            let statefulset_result: Result<StatefulSet, _> = parse_k8s_yaml(&contents);
            if let Ok(mut statefulset) = statefulset_result {
                if let Some(spec) = statefulset.spec.as_mut() {
                    if let Some(template_spec) = spec.template.spec.as_mut() {
                        for container in &mut template_spec.containers {
                            if container.image.as_deref().unwrap_or_default().contains(base_image) {
                                log::info!("Found target image in file: {:?}", entry.path());
                                container.image = Some(new_image.clone());
                                image_updated = true;
                            }
                            log::info!("Found target image in file: {:?}", entry.path());
                        }
                    }
                }
                log::info!("New StatefulSet: {:?}", &mut statefulset);
                if image_updated {
                    log::info!("Updating image in file: {:?}", entry.path());
                    let mut file = OpenOptions::new()
                        .write(true)
                        .truncate(true)
                        .open(entry.path())
                        .unwrap();
                    file.write_all(serde_yaml_ng::to_string(&statefulset).unwrap().as_bytes())
                        .unwrap();
                }
            }
            let deployment_result: Result<Deployment, _> = parse_k8s_yaml(&contents);
            match deployment_result {
                Ok(mut deployment) => {
                    log::info!("Deployment: {:?}", &deployment);
                    if let Some(spec) = deployment.spec.as_mut() {
                        if let Some(template_spec) = spec.template.spec.as_mut() {
                            for container in &mut template_spec.containers {
                                if container.image.as_deref().unwrap_or_default().contains(base_image) {
                                    log::info!("Found target image in file: {:?}", entry.path());
                                    container.image = Some(new_image.clone());
                                    image_updated = true;
                                }
                            }
                        }
                    }
                    if image_updated {
                        log::info!("Updating image in file: {:?}", entry.path());
                        let mut file = OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(entry.path())
                            .unwrap();
                        file.write_all(serde_yaml_ng::to_string(&deployment).unwrap().as_bytes())
                            .unwrap();
                    }
                }
                Err(e) => {
                    log::warn!("Skipping {:?}: not a valid Deployment ({}))", entry.path(), e);
                }
            }
        }
    }
}

fn stage_changes(repo: &Repository) -> Result<(), git2::Error> {
    let mut index = repo.index()?;
    index.add_all(["*"], IndexAddOption::DEFAULT, None)?;
    index.write()?;
    Ok(())
}

fn commit_changes<'a>(
    repo: &'a Repository,
    message: &str,
    commit_name: &str,
    commit_email: &str,
) -> Result<Commit<'a>, git2::Error> {
    let sig = Signature::now(commit_name, commit_email)?;
    let oid = repo.index()?.write_tree()?;
    let tree = repo.find_tree(oid)?;
    let parent_commit = find_last_commit(repo)?;
    let commit = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent_commit])?;
    repo.find_commit(commit)
}

fn find_last_commit(repo: &Repository) -> Result<Commit<'_>, git2::Error> {
    let obj = repo.head()?.resolve()?.peel(git2::ObjectType::Commit)?;
    obj.into_commit()
        .map_err(|_| git2::Error::from_str("Couldn't find commit"))
}

fn push_changes(repo: &Repository, access_token: &str) -> Result<(), git2::Error> {
    let mut cb = RemoteCallbacks::new();
    log::info!("Setting credentials");
    cb.credentials(move |_url, _username, _allowed_types| {
        Cred::userpass_plaintext("x-access-token", access_token)
    });
    log::info!("Setting credentials Done");

    let mut opts = PushOptions::new();
    opts.remote_callbacks(cb);

    let mut remote = repo.find_remote("origin")?;
    remote.push(&["refs/heads/main:refs/heads/main"], Some(&mut opts))?;
    Ok(())
}

pub async fn run_git_operations(workload: Workload) -> Result<(), Box<dyn Error + Send + Sync>> {
    match load_settings() {
        Ok(settings) => {
            log::info!("Settings: {:?}", settings);
            run_git_operations_internal(settings, workload).await
        }
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            Ok(())
        }
    }
}

async fn run_git_operations_internal(
    settings: Vec<GitopsConfig>,
    workload: Workload,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for gitops_config in settings {
        log::info!("Gitops config: {:?}", gitops_config);
        log::info!("Workload: {:?}", workload);
        if gitops_config.name.as_str() != workload.git_ops_repo.clone().unwrap_or_default().as_str()
        {
            log::info!(
                "Skipping gitops operation for repository: {}",
                gitops_config.name
            );
            continue;
        }
        let commit_name = gitops_config.commit_name;
        let commit_email = gitops_config.commit_email;
        let commit_message = gitops_config.commit_message;
        let repo_url = gitops_config.repository_url;
        let name = gitops_config.name;
        let access_token_env_name = gitops_config.access_token_env_name;
        let access_token = std::env::var(access_token_env_name).unwrap_or_default();
        log::info!("Access token: {}", access_token);
        let local_path = Path::new("/tmp/repos/").join(name);
        log::info!("Running git operations for repository: {}", repo_url);
        log::info!("Local path: {:?}", local_path);
        delete_local_repo()?;
        let repo = clone_or_open_repo(&repo_url, &local_path, &access_token)?;
        log::info!("Cloned Repo Complete");
        edit_files(&local_path, &workload);
        stage_changes(&repo)?;
        commit_changes(&repo, &commit_message, &commit_name, &commit_email)?;
        push_changes(&repo, &access_token)?;
        notify_commit(&workload).await?;
    }

    Ok(())
}
