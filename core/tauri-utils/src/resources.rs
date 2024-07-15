// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
  collections::HashMap,
  path::{Component, Path, PathBuf},
};

use walkdir::WalkDir;

/// Given a path (absolute or relative) to a resource file, returns the
/// relative path from the bundle resources directory where that resource
/// should be stored.
pub fn resource_relpath(path: &Path) -> PathBuf {
  let mut dest = PathBuf::new();
  for component in path.components() {
    match component {
      Component::Prefix(_) => {}
      Component::RootDir => dest.push("_root_"),
      Component::CurDir => {}
      Component::ParentDir => dest.push("_up_"),
      Component::Normal(string) => dest.push(string),
    }
  }
  dest
}

fn normalize(path: &Path) -> PathBuf {
  let mut dest = PathBuf::new();
  for component in path.components() {
    match component {
      Component::Prefix(_) => {}
      Component::RootDir => dest.push("/"),
      Component::CurDir => {}
      Component::ParentDir => dest.push(".."),
      Component::Normal(string) => dest.push(string),
    }
  }
  dest
}

/// Parses the external binaries to bundle, adding the target triple suffix to each of them.
pub fn external_binaries(external_binaries: &[String], target_triple: &str) -> Vec<String> {
  let mut paths = Vec::new();
  for curr_path in external_binaries {
    paths.push(format!(
      "{}-{}{}",
      curr_path,
      target_triple,
      if target_triple.contains("windows") {
        ".exe"
      } else {
        ""
      }
    ));
  }
  paths
}

/// Information for a resource.
#[derive(Debug)]
pub struct Resource {
  path: PathBuf,
  target: PathBuf,
}

impl Resource {
  /// The path of the resource.
  pub fn path(&self) -> &Path {
    &self.path
  }

  /// The target location of the resource.
  pub fn target(&self) -> &Path {
    &self.target
  }
}

#[derive(Debug)]
enum PatternIter<'a> {
  Slice(std::slice::Iter<'a, String>),
  Map(std::collections::hash_map::Iter<'a, String, String>),
}

/// A helper to iterate through resources.
pub struct ResourcePaths<'a> {
  iter: ResourcePathsIter<'a>,
}

impl<'a> ResourcePaths<'a> {
  /// Creates a new ResourcePaths from a slice of patterns to iterate
  pub fn new(patterns: &'a [String], allow_walk: bool) -> ResourcePaths<'a> {
    ResourcePaths {
      iter: ResourcePathsIter {
        pattern_iter: PatternIter::Slice(patterns.iter()),
        allow_walk,
        current_path: None,
        current_dest: None,
        walk_iter: None,
        glob_iter: None,
      },
    }
  }

  /// Creates a new ResourcePaths from a slice of patterns to iterate
  pub fn from_map(patterns: &'a HashMap<String, String>, allow_walk: bool) -> ResourcePaths<'a> {
    ResourcePaths {
      iter: ResourcePathsIter {
        pattern_iter: PatternIter::Map(patterns.iter()),
        allow_walk,
        current_path: None,
        current_dest: None,
        walk_iter: None,
        glob_iter: None,
      },
    }
  }

  /// Returns the resource iterator that yields the source and target paths.
  /// Needed when using [`Self::from_map`].
  pub fn iter(self) -> ResourcePathsIter<'a> {
    self.iter
  }
}

/// Iterator of a [`ResourcePaths`].
#[derive(Debug)]
pub struct ResourcePathsIter<'a> {
  /// the patterns to iterate.
  pattern_iter: PatternIter<'a>,
  /// whether the resource paths allows directories or not.
  allow_walk: bool,

  current_path: Option<PathBuf>,
  current_dest: Option<&'a str>,
  walk_iter: Option<walkdir::IntoIter>,
  glob_iter: Option<glob::Paths>,
}

impl<'a> ResourcePathsIter<'a> {
  fn next_glob_iter(&mut self) -> Option<crate::Result<Resource>> {
    let Some(entry) = self.glob_iter.as_mut().unwrap().next() else {
      return None;
    };

    let entry = match entry {
      Ok(entry) => entry,
      Err(err) => return Some(Err(err.into())),
    };

    self.current_path = Some(normalize(&entry));
    self.next_current_path()
  }

  fn next_walk_iter(&mut self) -> Option<crate::Result<Resource>> {
    let Some(entry) = self.walk_iter.as_mut().unwrap().next() else {
      return None;
    };

    let entry = match entry {
      Ok(entry) => entry,
      Err(err) => return Some(Err(err.into())),
    };

    self.current_path = Some(normalize(entry.path()));
    self.next_current_path()
  }

  fn next_current_path(&mut self) -> Option<crate::Result<Resource>> {
    // should be safe to unwrap since every call to `self.next_current_path()`
    // is preceeded with assignemt to `self.current_path`
    let path = self.current_path.take().unwrap();

    let is_dir = path.is_dir();

    if is_dir && !self.allow_walk {
      return Some(Err(crate::Error::NotAllowedToWalkDir(path.to_path_buf())));
    }

    if is_dir {
      if let None = self.walk_iter {
        self.walk_iter = Some(WalkDir::new(path).into_iter());
      }

      match self.next_walk_iter() {
        Some(resource) => Some(resource),
        None => self.next(),
      }
    } else {
      let resource = Resource {
        target: self
          .current_dest
          .map(|current_dest| {
            if current_dest.is_empty() {
              if let Some(file_name) = path.file_name() {
                return PathBuf::from(file_name);
              }
            }

            PathBuf::from(current_dest)
          })
          .unwrap_or_else(|| resource_relpath(&path)),
        path: path.to_path_buf(),
      };
      Some(Ok(resource))
    }
  }

  fn next_pattern(&mut self) -> Option<crate::Result<Resource>> {
    self.current_dest = None;
    self.current_path = None;

    let pattern = match &mut self.pattern_iter {
      PatternIter::Slice(iter) => match iter.next() {
        Some(pattern) => pattern,
        None => return None,
      },
      PatternIter::Map(iter) => match iter.next() {
        Some((pattern, dest)) => {
          self.current_dest = Some(dest.as_str());
          pattern
        }
        None => return None,
      },
    };

    if pattern.contains('*') {
      self.glob_iter = match glob::glob(pattern) {
        Ok(glob) => Some(glob),
        Err(error) => return Some(Err(error.into())),
      };
      match self.next_glob_iter() {
        Some(r) => return Some(r),
        None => {}
      };
    }

    self.current_path = Some(normalize(Path::new(pattern)));
    self.next_current_path()
  }
}

impl<'a> Iterator for ResourcePaths<'a> {
  type Item = crate::Result<PathBuf>;

  fn next(&mut self) -> Option<crate::Result<PathBuf>> {
    self.iter.next().map(|r| r.map(|res| res.path))
  }
}

impl<'a> Iterator for ResourcePathsIter<'a> {
  type Item = crate::Result<Resource>;

  fn next(&mut self) -> Option<crate::Result<Resource>> {
    if self.current_path.is_some() {
      return self.next_current_path();
    }

    if self.walk_iter.is_some() {
      match self.next_walk_iter() {
        Some(r) => return Some(r),
        None => {}
      };
    }

    if self.glob_iter.is_some() {
      match self.next_glob_iter() {
        Some(r) => return Some(r),
        None => {}
      };
    }

    self.next_pattern()
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use std::fs;
  use std::path::Path;

  impl PartialEq for Resource {
    fn eq(&self, other: &Self) -> bool {
      self.path == other.path && self.target == other.target
    }
  }

  fn expected_resources(resources: &[(&str, &str)]) -> Vec<Resource> {
    resources
      .into_iter()
      .map(|(path, target)| Resource {
        path: Path::new(path).components().collect(),
        target: Path::new(target).components().collect(),
      })
      .collect()
  }

  fn setup_test_dirs() {
    let mut random = [0; 1];
    getrandom::getrandom(&mut random).unwrap();

    let temp = std::env::temp_dir();
    let temp = temp.join(format!("tauri_resource_paths_iter_test_{}", random[0]));

    let _ = fs::remove_dir_all(&temp);
    let _ = fs::create_dir_all(&temp).unwrap();

    std::env::set_current_dir(&temp).unwrap();

    let paths = [
      Path::new("src-tauri/tauri.conf.json"),
      Path::new("src-tauri/some-other-json.json"),
      Path::new("src-tauri/Cargo.toml"),
      Path::new("src-tauri/Tauri.toml"),
      Path::new("src-tauri/build.rs"),
      Path::new("src/assets/javascript.svg"),
      Path::new("src/assets/tauri.svg"),
      Path::new("src/assets/rust.svg"),
      Path::new("src/index.html"),
      Path::new("src/style.css"),
      Path::new("src/script.js"),
    ];

    for path in paths {
      fs::create_dir_all(path.parent().unwrap()).unwrap();
      fs::write(path, "").unwrap();
    }
  }

  #[test]
  #[serial_test::serial]
  fn resource_paths_iter_slice_allow_walk() {
    setup_test_dirs();

    let dir = std::env::current_dir().unwrap().join("src-tauri");
    let _ = std::env::set_current_dir(dir);

    let resources = ResourcePaths::new(
      &[
        "../src/script.js".into(),
        "../src/assets".into(),
        "../src/index.html".into(),
        "*.toml".into(),
        "*.conf.json".into(),
      ],
      true,
    )
    .iter()
    .flatten()
    .collect::<Vec<_>>();

    let expected = expected_resources(&[
      ("../src/script.js", "_up_/src/script.js"),
      (
        "../src/assets/javascript.svg",
        "_up_/src/assets/javascript.svg",
      ),
      ("../src/assets/tauri.svg", "_up_/src/assets/tauri.svg"),
      ("../src/assets/rust.svg", "_up_/src/assets/rust.svg"),
      ("../src/index.html", "_up_/src/index.html"),
      ("Cargo.toml", "Cargo.toml"),
      ("Tauri.toml", "Tauri.toml"),
      ("tauri.conf.json", "tauri.conf.json"),
    ]);

    assert_eq!(resources.len(), expected.len());
    for resource in expected {
      if !resources.contains(&resource) {
        panic!("{resource:?} was expected but not found in {resources:?}");
      }
    }
  }

  #[test]
  #[serial_test::serial]
  fn resource_paths_iter_slice_no_walk() {
    setup_test_dirs();

    let dir = std::env::current_dir().unwrap().join("src-tauri");
    let _ = std::env::set_current_dir(dir);

    let resources = ResourcePaths::new(
      &[
        "../src/script.js".into(),
        "../src/assets".into(),
        "../src/index.html".into(),
        "*.toml".into(),
        "*.conf.json".into(),
      ],
      false,
    )
    .iter()
    .flatten()
    .collect::<Vec<_>>();

    let expected = expected_resources(&[
      ("../src/script.js", "_up_/src/script.js"),
      ("../src/index.html", "_up_/src/index.html"),
      ("Cargo.toml", "Cargo.toml"),
      ("Tauri.toml", "Tauri.toml"),
      ("tauri.conf.json", "tauri.conf.json"),
    ]);

    assert_eq!(resources.len(), expected.len());
    for resource in expected {
      if !resources.contains(&resource) {
        panic!("{resource:?} was expected but not found in {resources:?}");
      }
    }
  }

  #[test]
  #[serial_test::serial]
  fn resource_paths_iter_map_allow_walk() {
    setup_test_dirs();

    let dir = std::env::current_dir().unwrap().join("src-tauri");
    let _ = std::env::set_current_dir(dir);

    let resources = ResourcePaths::from_map(
      &std::collections::HashMap::from_iter([
        ("../src/script.js".into(), "main.js".into()),
        ("../src/assets".into(), "".into()),
        ("../src/index.html".into(), "frontend/index.html".into()),
        ("*.toml".into(), "".into()),
        ("*.conf.json".into(), "".into()),
      ]),
      true,
    )
    .iter()
    .flatten()
    .collect::<Vec<_>>();

    let expected = expected_resources(&[
      ("../src/script.js", "main.js"),
      ("../src/assets/javascript.svg", "javascript.svg"),
      ("../src/assets/tauri.svg", "tauri.svg"),
      ("../src/assets/rust.svg", "rust.svg"),
      ("../src/index.html", "frontend/index.html"),
      ("Cargo.toml", "Cargo.toml"),
      ("Tauri.toml", "Tauri.toml"),
      ("tauri.conf.json", "tauri.conf.json"),
    ]);

    assert_eq!(resources.len(), expected.len());
    for resource in expected {
      if !resources.contains(&resource) {
        panic!("{resource:?} was expected but not found in {resources:?}");
      }
    }
  }

  #[test]
  #[serial_test::serial]
  fn resource_paths_iter_map_no_walk() {
    setup_test_dirs();

    let dir = std::env::current_dir().unwrap().join("src-tauri");
    let _ = std::env::set_current_dir(dir);

    let resources = ResourcePaths::from_map(
      &std::collections::HashMap::from_iter([
        ("../src/script.js".into(), "main.js".into()),
        ("../src/assets".into(), "".into()),
        ("../src/index.html".into(), "frontend/index.html".into()),
        ("*.toml".into(), "".into()),
        ("*.conf.json".into(), "".into()),
      ]),
      false,
    )
    .iter()
    .flatten()
    .collect::<Vec<_>>();

    let expected = expected_resources(&[
      ("../src/script.js", "main.js"),
      ("../src/index.html", "frontend/index.html"),
      ("Cargo.toml", "Cargo.toml"),
      ("Tauri.toml", "Tauri.toml"),
      ("tauri.conf.json", "tauri.conf.json"),
    ]);

    assert_eq!(resources.len(), expected.len());
    for resource in expected {
      if !resources.contains(&resource) {
        panic!("{resource:?} was expected but not found in {resources:?}");
      }
    }
  }
}
