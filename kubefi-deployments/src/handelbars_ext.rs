use std::fs::{DirEntry, File};
use std::io::{BufRead, BufReader, Error};
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
    let path_param = read_path(h)?;
    let indent_param = read_indent(h)?;
    let excluded_files = read_exclude_filter(h, ctx)?;

    let template_dir_path = format!("./templates/{}", path_param);
    let path = Path::new(&template_dir_path);

    if path.exists() {
        let result = fs::read_dir(path)?
            .filter(|entry| !excluded(entry, &excluded_files))
            .map(|entry| {
                entry.map(|e| {
                    let p = e.path();
                    let f =
                        File::open(&p).unwrap_or_else(|_| panic!("Cannot open file path: {:?}", p));
                    let file = BufReader::new(&f);
                    let content = file
                        .lines()
                        .map(|l| format_line(indent_param, &f, l))
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
            "templates path {:?} does not exist",
            &path
        )))
    }
}

fn excluded(entry: &Result<DirEntry, Error>, files: &[String]) -> bool {
    entry
        .as_ref()
        .map(|e| {
            let p = e.path();
            let path = &p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            files.iter().any(|f| f == path)
        })
        .unwrap_or(false)
}

fn read_exclude_filter(h: &Helper, ctx: &Context) -> Result<Vec<String>, RenderError> {
    h.param(2).map(|v| {
        let key = v.render();
        let array = ctx.data().get(&key).and_then(|v| v.as_array());
        array.ok_or_else(|| {
            let error = format!("get_files 'exclude_filter' parameter is set to {}, but its value is not found in the Template context", key);
            RenderError::new(error)
        }).map(
            |files| {
                let file_names: Vec<String> = files.iter().flat_map(|f| f.as_str()).map(String::from).collect();
                file_names
            }
        )
    }).unwrap_or_else(|| Ok(vec![]))
}

fn read_indent(h: &Helper) -> Result<usize, RenderError> {
    h.param(1)
        .ok_or_else(|| RenderError::new("get_files 'indent' parameter at index 1 is missing"))?
        .value()
        .as_u64()
        .ok_or_else(|| RenderError::new("get_files 'indent' parameter expected to be unsigned int"))
        .map(|v| v as usize)
}

fn read_path(h: &Helper) -> Result<String, RenderError> {
    h.param(0)
        .ok_or_else(|| RenderError::new("get_files 'path' parameter at index 0 is missing"))
        .map(|v| v.render())
}

fn format_line(indent_param: usize, f: &File, l: Result<String, Error>) -> String {
    format!(
        "{}{}\n",
        format_args!("{: >1$}", "", indent_param),
        l.unwrap_or_else(|_| panic!("Failed to read a line from file: {:?}", &f))
    )
}

#[cfg(test)]
mod tests {
    use crate::crd::PodResources;
    use crate::crd::Resources;
    use crate::crd::{NiFiDeploymentSpec, ZooKeeper};
    use std::path::Path;

    use crate::template::Template;

    #[test]
    fn print_configmap() {
        let config = super::super::config::read_nifi_config().expect("Failed to load config");
        let template = Template::new(Path::new("./templates"), config)
            .expect("Failed to create template engine");
        let name = "test".to_string();
        let content = template
            .nifi_configmap(&name, "test", &(3 as u8), &None)
            .expect("Failed to render configmap template");
        println!("content:\n{}", content.unwrap())
    }

    #[test]
    fn print_statefulset() {
        let config = super::super::config::read_nifi_config().expect("Failed to load config");
        let template = Template::new(Path::new("./templates"), config)
            .expect("Failed to create template engine");
        let name = "test".to_string();
        let res = Some(Resources {
            jvm_heap_size: None,
            requests: Some(PodResources {
                cpu: Some("rrrr".to_string()),
                memory: Some("rrr_mmmm".to_string()),
            }),
            limits: Some(PodResources {
                cpu: Some("llll".to_string()),
                memory: Some("llll_mmm".to_string()),
            }),
        });
        let spec = NiFiDeploymentSpec {
            nifi_replicas: 2,
            zk: ZooKeeper {
                replicas: 2,
                image: None,
            },
            image: None,
            storage_class: None,
            ldap: None,
            logging_config_map: None,
            nifi_resources: res,
            ingress: None,
        };
        let content = template
            .nifi_statefulset(&name, &spec)
            .expect("Failed to render configmap template");
        println!("content:\n{}", content.unwrap())
    }
}
