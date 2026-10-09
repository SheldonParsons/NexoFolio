//! Ordered lookup views over the same evidence; indexing never drops values.
use super::SampleField;
use nexofolio_common::EndpointId;
use nexofolio_contracts::endpoint::FieldLocation;
use std::collections::HashMap;

pub struct SignalIndex<'a> {
    signals: &'a [SampleField],
    endpoints: HashMap<EndpointId, Vec<usize>>,
    fields: HashMap<(EndpointId, FieldLocation, &'a str), Vec<usize>>,
    names: HashMap<&'a str, Vec<usize>>,
    values: HashMap<&'a str, Vec<usize>>,
    named_values: HashMap<(&'a str, &'a str), Vec<usize>>,
    records: HashMap<(EndpointId, &'a str, &'a str), Vec<usize>>,
    examples: HashMap<(EndpointId, &'a str), Vec<usize>>,
}
impl<'a> SignalIndex<'a> {
    pub fn new(signals: &'a [SampleField]) -> Self {
        let mut index = Self {
            signals,
            endpoints: HashMap::new(),
            fields: HashMap::new(),
            names: HashMap::new(),
            values: HashMap::new(),
            named_values: HashMap::new(),
            records: HashMap::new(),
            examples: HashMap::new(),
        };
        for (i, s) in signals.iter().enumerate() {
            index.endpoints.entry(s.endpoint).or_default().push(i);
            index
                .fields
                .entry((s.endpoint, s.location, &s.path))
                .or_default()
                .push(i);
            index.names.entry(&s.name).or_default().push(i);
            index.values.entry(&s.value).or_default().push(i);
            index
                .named_values
                .entry((&s.name, &s.value))
                .or_default()
                .push(i);
            index
                .records
                .entry((s.endpoint, &s.example, &s.row))
                .or_default()
                .push(i);
            index
                .examples
                .entry((s.endpoint, &s.example))
                .or_default()
                .push(i);
        }
        index
    }
    fn view<'s>(
        &'s self,
        indices: Option<&'s Vec<usize>>,
    ) -> impl Iterator<Item = &'a SampleField> + 's {
        indices
            .into_iter()
            .flat_map(|v| v.iter())
            .map(|i| &self.signals[*i])
    }
    pub fn all(&self) -> &'a [SampleField] {
        self.signals
    }
    pub fn endpoint(&self, id: EndpointId) -> impl Iterator<Item = &'a SampleField> + '_ {
        self.view(self.endpoints.get(&id))
    }
    pub fn field<'s>(
        &'s self,
        id: EndpointId,
        location: FieldLocation,
        path: &'s str,
    ) -> impl Iterator<Item = &'a SampleField> + 's {
        self.view(self.fields.get(&(id, location, path)))
    }
    pub fn name(&self, name: &str) -> impl Iterator<Item = &'a SampleField> + '_ {
        self.view(self.names.get(name))
    }
    pub fn value(&self, value: &str) -> impl Iterator<Item = &'a SampleField> + '_ {
        self.view(self.values.get(value))
    }
    pub fn named_value<'s>(
        &'s self,
        name: &'s str,
        value: &'s str,
    ) -> impl Iterator<Item = &'a SampleField> + 's {
        self.view(self.named_values.get(&(name, value)))
    }
    pub fn record<'s>(&'s self, s: &'s SampleField) -> impl Iterator<Item = &'a SampleField> + 's {
        self.view(self.records.get(&(s.endpoint, &s.example, &s.row)))
    }
    pub fn example<'s>(&'s self, s: &'s SampleField) -> impl Iterator<Item = &'a SampleField> + 's {
        self.view(self.examples.get(&(s.endpoint, &s.example)))
    }
}
