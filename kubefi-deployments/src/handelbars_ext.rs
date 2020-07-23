use std::{fs, io};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use handlebars::{Context, Handlebars, Helper, HelperResult, Output, RenderContext, RenderError};

pub fn get_files_helper(h: &Helper, _: &Handlebars, _: &Context, _: &mut RenderContext, out: &mut dyn Output) -> HelperResult {
    let path_param = h.param(0)
        .ok_or(RenderError::new("get_files 'path' parameter at index 0 is missing"))?
        .render();
    let ident_param = h.param(1)
        .ok_or(RenderError::new("get_files 'ident' parameter at index 1 is missing"))?
        .value().as_u64()
        .ok_or(RenderError::new("get_files 'ident' parameter expected to be unsigned int"))? as usize;
    let template_dir_path = format!("./templates/{}", path_param);
    let path = Path::new(&template_dir_path);

    if path.exists() {
        let paths = fs::read_dir(path)?
            .map(|res| res.map(|e| {
                let p = e.path();
                let f = File::open(&p).unwrap();
                let file = BufReader::new(&f);
                let content = file.lines()
                    .map(|l|
                        format!("{}{}\n", format_args!("{: >1$}", "", ident_param), l.unwrap())
                    )
                    .collect::<String>();
                format!("  {}:\n{}\n\n", &p.file_name().unwrap().to_str().unwrap(), content)
            })).collect::<Result<String, io::Error>>()?;

        out.write(paths.as_str())?;
        Ok(())
    } else {
        Err(RenderError::new(format!("templates path {:?} does not exist", &path)))
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