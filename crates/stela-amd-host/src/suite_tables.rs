//! Visible function tables of the suite's cases, as the Mac's runner fills
//! them: the case's visible functions link into the pipeline stage, and
//! each table holds the pipeline's handles of the functions it names, bound
//! as a buffer at its index.
use super::suite::Runner;
use serde_json::Value;
use stela_driver_gpu::metal::{
    command_buffer::CommandBuffer,
    device::{ComputePipelineId, FunctionId, LibraryId, RenderPipelineId},
    device::pipelines::LinkedFunctions,
    error::{Error, Result},
    shader::Stage,
};

/// A stage's linked functions, by name.
pub type Linked = Vec<(String, FunctionId)>;

/// Whether this runner links and fills what `spec` asks of a stage whose
/// keys start with `prefix` ("" for compute, "vertex_" or "fragment_").
pub fn supported(spec: &Value, prefix: &str) -> bool {
    let names = |key: &str| {
        spec[format!("{prefix}{key}")]
            .as_array()
            .into_iter()
            .flatten()
            .all(|entry| {
                // A function by name, or from another library.
                entry.is_string()
                    || entry.as_object().is_some_and(|e| {
                        e.keys().all(|k| k == "library" || k == "name")
                    })
            })
    };
    let tables = spec[format!("{prefix}{}", if prefix.is_empty() { "function_tables" } else { "tables" })]
        .as_array()
        .into_iter()
        .flatten()
        .all(|table| {
            table
                .as_object()
                .is_some_and(|t| t.keys().all(|k| k == "functions" || k == "index"))
        });
    names("visible_functions") && names("private_functions") && names("binary_functions") && tables
}

impl Runner<'_> {
    /// The linked functions of a stage: `{prefix}visible_functions` and
    /// `{prefix}binary_functions` its tables call, and
    /// `{prefix}private_functions` only direct calls reach.
    pub(crate) fn linked_functions(
        &mut self,
        library: LibraryId,
        spec: &Value,
        prefix: &str,
    ) -> Result<(LinkedFunctions, Linked)> {
        let mut linked = LinkedFunctions::default();
        let mut names = Vec::new();
        for (key, private) in [
            ("visible_functions", false),
            ("binary_functions", false),
            ("private_functions", true),
        ] {
            for name in spec[format!("{prefix}{key}")].as_array().into_iter().flatten() {
                let (from, name) = match name.as_str() {
                    Some(name) => (library, name),
                    None => (
                        self.library(name["library"].as_str().unwrap_or_default())?,
                        name["name"].as_str().unwrap_or_default(),
                    ),
                };
                let id = self.device.new_function(from, name, Some(&[]))?;
                if private {
                    linked.private_functions.push(id);
                } else {
                    linked.functions.push(id);
                }
                names.push((name.to_owned(), id));
            }
        }
        if let Some(depth) = spec[format!("{prefix}call_stack_depth")].as_u64() {
            linked.max_call_stack_depth = depth as u32;
        }
        Ok((linked, names))
    }

    /// Fills and binds the tables `tables` names, each handle from `handle`.
    fn bind_tables(
        &mut self,
        tables: &Value,
        linked: &Linked,
        handle: &mut dyn FnMut(&mut Self, FunctionId) -> Result<Option<stela_driver_gpu::metal::device::pipelines::FunctionHandle>>,
        commands: &mut CommandBuffer,
        stage: Stage,
    ) -> Result<()> {
        for table in tables.as_array().into_iter().flatten() {
            let names = table["functions"].as_array().map_or(&[][..], Vec::as_slice);
            let id = self.device.new_visible_function_table(names.len() as u64)?;
            self.tables.push(id);
            let mut handles = Vec::new();
            for name in names {
                let name = name.as_str().unwrap_or_default();
                let function = linked
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, id)| *id)
                    .ok_or(Error::Invalid("a table names a function the stage does not link"))?;
                handles.push(Some(
                    handle(self, function)?.ok_or(Error::Invalid("no function handle"))?,
                ));
            }
            self.device.set_visible_functions(id, 0, &handles)?;
            let reference = self.device.buffer(id)?;
            let index = table["index"].as_u64().unwrap_or(0) as u32;
            if stage == Stage::Kernel {
                commands.set_compute_buffer(Some(reference), 0, index)?;
            } else {
                commands.set_render_buffer(stage, Some(reference), 0, index)?;
            }
        }
        Ok(())
    }

    /// Binds a compute pipeline's tables.
    pub(crate) fn bind_compute_tables(
        &mut self,
        spec: &Value,
        pipeline: ComputePipelineId,
        linked: &Linked,
        commands: &mut CommandBuffer,
    ) -> Result<()> {
        self.bind_tables(
            &spec["function_tables"],
            linked,
            &mut |runner, function| runner.device.compute_function_handle(pipeline, function),
            commands,
            Stage::Kernel,
        )
    }

    /// Binds a render pipeline's vertex and fragment tables.
    pub(crate) fn bind_render_tables(
        &mut self,
        render: &Value,
        pipeline: RenderPipelineId,
        linked: [&Linked; 2],
        commands: &mut CommandBuffer,
    ) -> Result<()> {
        for (key, stage, linked) in [
            ("vertex_tables", Stage::Vertex, linked[0]),
            ("fragment_tables", Stage::Fragment, linked[1]),
        ] {
            self.bind_tables(
                &render[key],
                linked,
                &mut |runner, function| runner.device.render_function_handle(pipeline, stage, function),
                commands,
                stage,
            )?;
        }
        Ok(())
    }
}
