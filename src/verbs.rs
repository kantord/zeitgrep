use anyhow::{Context, Result};
use minijinja::{Environment, Template};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbTemplate {
    pub name: String,
    pub content: String,
    pub source: TemplateSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TemplateSource {
    BuiltIn,
    UserConfig,
    Local,
}

pub struct VerbManager {
    env: Environment<'static>,
    templates: Vec<VerbTemplate>,
}

impl VerbManager {
    pub fn new() -> Result<Self> {
        let mut env = Environment::new();
        let mut templates = Vec::new();

        Self::load_builtin_templates(&mut templates)?;
        Self::load_user_config_templates(&mut templates)?;
        Self::load_local_templates(&mut templates)?;

        for template in &templates {
            env.add_template(&template.name, &template.content)
                .with_context(|| format!("Failed to add template: {}", template.name))?;
        }

        Ok(Self { env, templates })
    }

    fn load_builtin_templates(templates: &mut Vec<VerbTemplate>) -> Result<()> {
        let builtin_dir = PathBuf::from("templates/verbs");
        if !builtin_dir.exists() {
            return Ok(());
        }

        for entry in WalkDir::new(builtin_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "jinja"))
        {
            let content = std::fs::read_to_string(entry.path())
                .with_context(|| format!("Failed to read template: {:?}", entry.path()))?;
            
            let name = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow::anyhow!("Invalid template filename"))?;

            templates.push(VerbTemplate {
                name: name.to_string(),
                content,
                source: TemplateSource::BuiltIn,
            });
        }

        Ok(())
    }

    fn load_user_config_templates(templates: &mut Vec<VerbTemplate>) -> Result<()> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
            .join("lakonik/verbs");

        if !config_dir.exists() {
            return Ok(());
        }

        Self::load_templates_from_dir(&config_dir, templates, TemplateSource::UserConfig)
    }

    fn load_local_templates(templates: &mut Vec<VerbTemplate>) -> Result<()> {
        let local_dir = PathBuf::from(".lakonik/verbs");
        if !local_dir.exists() {
            return Ok(());
        }

        Self::load_templates_from_dir(&local_dir, templates, TemplateSource::Local)
    }

    fn load_templates_from_dir(
        dir: &Path,
        templates: &mut Vec<VerbTemplate>,
        source: TemplateSource,
    ) -> Result<()> {
        for entry in WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "jinja"))
        {
            let content = std::fs::read_to_string(entry.path())
                .with_context(|| format!("Failed to read template: {:?}", entry.path()))?;
            
            let name = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow::anyhow!("Invalid template filename"))?;

            templates.push(VerbTemplate {
                name: name.to_string(),
                content,
                source,
            });
        }

        Ok(())
    }

    pub fn get_template(&self, name: &str) -> Option<&Template> {
        self.env.get_template(name)
    }

    pub fn list_templates(&self) -> &[VerbTemplate] {
        &self.templates
    }

    pub fn render_template(&self, name: &str, context: &serde_json::Value) -> Result<String> {
        let template = self
            .get_template(name)
            .ok_or_else(|| anyhow::anyhow!("Template not found: {}", name))?;

        template
            .render(context)
            .map_err(|e| anyhow::anyhow!("Template rendering error: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    fn create_test_template(dir: &Path, name: &str, content: &str) -> Result<()> {
        fs::create_dir_all(dir)?;
        fs::write(dir.join(format!("{}.jinja", name)), content)?;
        Ok(())
    }

    #[test]
    fn test_verb_manager_creation() -> Result<()> {
        let manager = VerbManager::new()?;
        assert!(manager.templates.is_empty());
        Ok(())
    }

    #[test]
    fn test_local_template_loading() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let verbs_dir = temp_dir.path().join(".lakonik/verbs");
        
        create_test_template(
            &verbs_dir,
            "test",
            "Hello {{ name }}!",
        )?;

        let manager = VerbManager::new()?;
        let templates = manager.list_templates();
        
        assert!(!templates.is_empty());
        assert!(templates.iter().any(|t| t.name == "test"));
        
        let result = manager.render_template("test", &serde_json::json!({
            "name": "World"
        }))?;
        
        assert_eq!(result, "Hello World!");
        
        Ok(())
    }

    #[test]
    fn test_template_not_found() -> Result<()> {
        let manager = VerbManager::new()?;
        let result = manager.render_template("nonexistent", &serde_json::json!({}));
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_template_rendering_error() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let verbs_dir = temp_dir.path().join(".lakonik/verbs");
        
        create_test_template(
            &verbs_dir,
            "invalid",
            "{{ invalid_syntax }}",
        )?;

        let manager = VerbManager::new()?;
        let result = manager.render_template("invalid", &serde_json::json!({}));
        assert!(result.is_err());
        
        Ok(())
    }
} 