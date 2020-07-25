use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::{fs, io};

use handlebars::{Context, Handlebars, Helper, HelperResult, Output, RenderContext, RenderError};

pub fn get_files_helper(
    h: &Helper,
    hs: &Handlebars,
    ctx: &Context,
    _: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    let path_param = h
        .param(0)
        .ok_or_else(|| RenderError::new("get_files 'path' parameter at index 0 is missing"))?
        .render();
    let ident_param = h
        .param(1)
        .ok_or_else(|| RenderError::new("get_files 'ident' parameter at index 1 is missing"))?
        .value()
        .as_u64()
        .ok_or_else(|| {
            RenderError::new("get_files 'ident' parameter expected to be unsigned int")
        })? as usize;
    let template_dir_path = format!("./templates/{}", path_param);
    let path = Path::new(&template_dir_path);

    if path.exists() {
        let result: String = fs::read_dir(path)?
            .map(|entry| {
                entry.map(|e| {
                    let p = e.path();
                    let f =
                        File::open(&p).unwrap_or_else(|_| panic!("Cannot open file path: {:?}", p));
                    let file = BufReader::new(&f);
                    let content = file
                        .lines()
                        .map(|l| {
                            format!(
                                "{}{}\n",
                                format_args!("{: >1$}", "", ident_param),
                                l.unwrap_or_else(|_| panic!(
                                    "Failed to read a line from file: {:?}",
                                    f
                                ))
                            )
                        })
                        .collect::<String>();
                    let file_name = &p.file_name().and_then(|n| n.to_str()).unwrap_or_else(|| {
                        panic!("Failed to read file name: {:?}", &p.file_name())
                    });
                    format!("  {}: |-\n{}\n", file_name, content)
                })
            })
            .collect::<Result<String, io::Error>>()?;

        let rendered = hs
            .render_template(result.as_str(), ctx.data())
            .map_err(|e| RenderError::from_error("Failed to render get_files content", e))?;
        out.write(rendered.as_str())?;
        Ok(())
    } else {
        Err(RenderError::new(format!(
            "templates path '{:?}' does not exist",
            &path
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::template::Template;

    #[test]
    fn print_template() {
        let config = super::super::operator_config::read_config().expect("Failed to load config");
        let template = Template::new(Path::new("./templates"), config)
            .expect("Failed to create template engine");
        let name = "test".to_string();
        let content = template
            .configmap(&name)
            .expect("Failed to render configmap template");
        println!("content:\n{}", content.unwrap())
    }
}
