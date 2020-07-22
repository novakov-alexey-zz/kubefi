use std::{fs, io};
use std::path::Path;

use handlebars::{Context, Handlebars, Helper, HelperResult, Output, RenderContext, RenderError};

pub fn get_files_helper(h: &Helper, _: &Handlebars, _: &Context, _: &mut RenderContext, out: &mut dyn Output) -> HelperResult {
    let path_str = h.param(0)
        .ok_or(RenderError::new("get_files parameter is missing"))?
        .render();
    let abs_path = format!("./templates/{}", path_str);
    let path = Path::new(&abs_path);

    if path.exists() {
        let paths = fs::read_dir(path)?
            .map(|res| res.map(|e| {
                let p = e.path();
                let file_name = &p.file_name().unwrap();
                let content = fs::read_to_string(&p).unwrap_or("".to_string());
                format!("{:?}:\n  {:?}", file_name, content)
            })).collect::<Result<String, io::Error>>()?;

        out.write(paths.as_str())?;
        Ok(())
    } else {
        Err(RenderError::new(format!("path {:?} does not exist", &path)))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::template::Template;

    #[test]
    fn print_template() {
        let template = Template::new(Path::new("./templates")).expect("Failed to create template engine");
        let name = "test".to_string();
        let content = template.configmap(&name).expect("Failed to render configmap template");
        println!("content:\n {}", content)
    }
}