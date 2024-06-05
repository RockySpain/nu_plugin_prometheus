use nu_protocol::{record, Record, Span, Value};
use prometheus_http_query::response::{InstantVector, RangeVector, Sample};
use std::collections::HashMap;

pub(crate) fn add_labels(record: &mut Record, metric: &HashMap<String, String>, flatten: bool) {
    if flatten {
        for (name, label) in metric {
            if name == "__name__" {
                continue;
            }

            record.push(name, Value::string(label, Span::unknown()));
        }
    } else {
        let mut labels = Record::new();
        for (name, label) in metric {
            if name == "__name__" {
                continue;
            }
            labels.push(name, Value::string(label, Span::unknown()));
        }

        record.insert("labels", Value::record(labels, Span::unknown()));
    }
}

pub(crate) fn matrix_to_value(matrix: &[RangeVector], flatten: bool) -> Value {
    let records = matrix
        .iter()
        .map(|rv| {
            let metric = rv.metric();
            let values = rv.samples().iter().map(scalar_to_value).collect();

            let name = metric
                .get("__name__")
                .cloned()
                .unwrap_or("[UNKNOWN]".to_string());

            let mut record = record! {
                "name" => Value::string(name, Span::unknown()),
            };

            add_labels(&mut record, metric, flatten);

            record.insert("values", Value::list(values, Span::unknown()));

            Value::record(record, Span::unknown())
        })
        .collect();

    Value::list(records, Span::unknown())
}

pub(crate) fn scalar_to_value(scalar: &Sample) -> Value {
    Value::record(
        record! {
            "value" => Value::float(scalar.value(), Span::unknown()),
            "timestamp" => Value::float(scalar.timestamp(), Span::unknown())
        },
        Span::unknown(),
    )
}

pub(crate) fn vector_to_value(vector: &[InstantVector], flatten: bool) -> Value {
    let records = vector
        .iter()
        .map(|iv| {
            let metric = iv.metric();

            let name = metric
                .get("__name__")
                .cloned()
                .unwrap_or("[UNKNOWN]".to_string());

            let mut record = record! {
                "name" => Value::string(name, Span::unknown()),
            };

            add_labels(&mut record, metric, flatten);

            let value = Value::float(iv.sample().value(), Span::unknown());
            record.insert("value", value);

            let timestamp = Value::float(iv.sample().timestamp(), Span::unknown());
            record.insert("timestamp", timestamp);

            Value::record(record, Span::unknown())
        })
        .collect();

    Value::list(records, Span::unknown())
}

#[cfg(test)]
mod test {
    use nu_protocol::{record, Span, Value};
    use prometheus_http_query::response::{InstantVector, RangeVector, Sample};
    use std::collections::HashMap;

    #[test]
    fn add_labels_flatten() {
        let mut metric = HashMap::new();
        metric.insert("job".into(), "prometheus".into());
        metric.insert("instance".into(), "localhost:9090".into());

        let mut record = record! {};

        super::add_labels(&mut record, &metric, true);

        assert_eq!(
            Value::string("prometheus", Span::unknown()),
            record.get("job").unwrap().clone()
        );

        assert_eq!(
            Value::string("localhost:9090", Span::unknown()),
            record.get("instance").unwrap().clone()
        );
    }

    #[test]
    fn add_labels_no_flatten() {
        let mut metric = HashMap::new();
        metric.insert("job".into(), "prometheus".into());
        metric.insert("instance".into(), "localhost:9090".into());

        let mut record = record! {};

        super::add_labels(&mut record, &metric, false);

        let expected = Value::record(
            record! {
                "job" => Value::string("prometheus", Span::unknown()),
                "instance" => Value::string("localhost:9090", Span::unknown()),
            },
            Span::unknown(),
        );

        assert_eq!(expected, record.get("labels").unwrap().clone());
    }

    #[test]
    fn matrix_to_value() {
        let data = r#"[
         {
            "metric" : {
               "__name__" : "up",
               "job" : "prometheus",
               "instance" : "localhost:9090"
            },
            "values" : [
               [ 1435781430.781, "1" ],
               [ 1435781445.781, "1" ],
               [ 1435781460.781, "1" ]
            ]
         },
         {
            "metric" : {
               "__name__" : "up",
               "job" : "node",
               "instance" : "localhost:9091"
            },
            "values" : [
               [ 1435781430.781, "0" ],
               [ 1435781445.781, "0" ],
               [ 1435781460.781, "1" ]
            ]
         }
      ]"#
        .as_bytes();
        let matrix: Vec<RangeVector> = serde_json::from_slice(data).unwrap();

        let result = super::matrix_to_value(&matrix, false);

        let record = result
            .clone()
            .into_list()
            .unwrap()
            .first()
            .unwrap()
            .clone()
            .into_record()
            .unwrap();

        assert_eq!("up", record.get("name").unwrap().as_str().unwrap());

        let labels = record.get("labels").unwrap().as_record().unwrap();

        assert_eq!("prometheus", labels.get("job").unwrap().as_str().unwrap());

        let values = record.get("values").unwrap().as_list().unwrap();

        assert_eq!(3, values.len());
    }

    #[test]
    fn scalar_to_value() {
        let data = r#"[1716956024.754,"1"]"#.as_bytes();
        let scalar: Sample = serde_json::from_slice(data).unwrap();

        let result = super::scalar_to_value(&scalar).into_record().unwrap();

        assert_eq!(1.0, result.get("value").unwrap().as_f64().unwrap());
        assert_eq!(
            1716956024,
            result.get("timestamp").unwrap().as_f64().unwrap() as u64
        );
    }

    #[test]
    fn vector_to_value() {
        let data = r#"[{"metric":{"__name__":"up","instance":"target.example","job":"job name"},"value":[1716956024.754,"1"]}]"#.as_bytes();
        let vector: Vec<InstantVector> = serde_json::from_slice(data).unwrap();

        let result = super::vector_to_value(&vector, false).into_list().unwrap();

        let record = result.first().unwrap().as_record().unwrap();

        assert_eq!("up", record.get("name").unwrap().as_str().unwrap());

        let labels = record.get("labels").unwrap().as_record().unwrap();

        assert_eq!("job name", labels.get("job").unwrap().as_str().unwrap());

        let value = record.get("value").unwrap().as_f64().unwrap();

        assert_eq!(1.0, value);

        let timestamp = record.get("timestamp").unwrap().as_f64().unwrap();

        assert_eq!(1716956024, timestamp as u64);
    }
}
