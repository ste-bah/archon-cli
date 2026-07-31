python3 - <<'PY'
import datetime
import hashlib
import itertools
import json
import pathlib
import sys

def parse_timestamp(value):
    # Accept the common serialized instant shapes without adding a dependency:
    # RFC3339/ISO-8601 (with 'Z' or offset), plain date, and epoch seconds/millis.
    # Returns an aware UTC datetime, or None when unparseable.
    if isinstance(value, (int, float)):
        seconds = float(value)
        if seconds > 1e11:  # milliseconds
            seconds /= 1000.0
        try:
            return datetime.datetime.fromtimestamp(seconds, datetime.timezone.utc)
        except (OverflowError, OSError, ValueError):
            return None
    text = str(value).strip()
    if not text:
        return None
    normalized_text = text[:-1] + '+00:00' if text.endswith('Z') else text
    try:
        parsed = datetime.datetime.fromisoformat(normalized_text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=datetime.timezone.utc)
    return parsed.astimezone(datetime.timezone.utc)

root = pathlib.Path(__PROJECT_ROOT__)
contract = json.loads(__CONTRACT_JSON__)
failures = []

# Step-variety bounds for the synthetic-series check. These live in the ENGINE,
# not in task specs, on purpose: every numeric threshold written into a spec
# becomes the number an agent fabricates to (a `row_count: 200` minimum produced
# a payload with exactly 200 forged rows). An agent reading its task cannot see
# these, and satisfying them requires genuinely varied data rather than padding.
step_variety_min_rows = int(contract.get('step_variety_min_rows') or 20)
step_variety_min_ratio = int(contract.get('step_variety_min_percent') or 20) / 100.0

def resolve(value):
    path = pathlib.Path(value)
    return path if path.is_absolute() else root / path

def get_field(value, path, default=None):
    if not path:
        return default
    current = value
    for part in str(path).split('.'):
        if not isinstance(current, dict) or part not in current:
            return default
        current = current[part]
    return current

def normalized(value):
    return str(value if value is not None else '').strip().lower()

def load_json(path, label):
    if not path.is_file() or path.stat().st_size == 0:
        failures.append(f'{label} missing or empty: {path}')
        return None
    try:
        return json.loads(path.read_text())
    except Exception as error:
        failures.append(f'{label} is not valid JSON: {path}: {error}')
        return None

def load_payload(path, payload_format):
    if not path.is_file() or path.stat().st_size == 0:
        failures.append(f'payload missing or empty: {path}')
        return []
    try:
        if payload_format == 'jsonl':
            return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        value = json.loads(path.read_text())
        return value if isinstance(value, list) else [value]
    except Exception as error:
        failures.append(f'payload cannot be parsed as {payload_format}: {path}: {error}')
        return []

def internal_consistency(value, label):
    if not isinstance(value, dict):
        return
    status_field = contract.get('validation_status_field')
    checks_field = contract.get('validation_checks_field')
    check_status_field = contract.get('validation_check_status_field')
    failed_values = {normalized(item) for item in contract.get('validation_failed_values', [])}
    passed_values = {normalized(item) for item in contract.get('validation_passed_values', [])}
    if not status_field or not checks_field or not check_status_field:
        return
    checks = get_field(value, checks_field, [])
    if not isinstance(checks, list):
        failures.append(f'{label} checks field is not an array')
        return
    failed_checks = [
        check for check in checks
        if normalized(get_field(check, check_status_field)) in failed_values
    ]
    overall = normalized(get_field(value, status_field))
    if failed_checks and overall in passed_values:
        failures.append(
            f'{label} is internally inconsistent: overall={overall} with {len(failed_checks)} failed check(s)'
        )

import glob as _glob
import re as _re

def has_placeholder(text):
    return '<' in str(text) and '>' in str(text)

def source_instances(art_field, source_path_value, records_field):
    # Precise per-instance resolution: instances are the entries of a declared
    # source-of-truth collection (e.g. the registry). Each entry names its own
    # artifact via art_field. Ignores filesystem orphans (files not backed by an
    # entry are not THIS contract's deliverables); detects entries whose artifact
    # is missing. Zero entries -> zero required instances (vacuous).
    source = load_json(resolve(source_path_value), 'declared instance source')
    records = get_field(source, records_field, {}) if source else {}
    if isinstance(records, dict):
        record_iter = list(records.items())
    elif isinstance(records, list):
        record_iter = list(enumerate(records))
    else:
        failures.append(f'instance source records field is not a collection: {records_field}')
        record_iter = []
    paths = []
    for key, record in record_iter:
        ref = get_field(record, art_field) if isinstance(record, dict) else None
        if not ref:
            failures.append(f'source entry {key} has no artifact reference field {art_field}')
            continue
        paths.append(resolve(ref))
    return paths

def glob_instances(pattern_text):
    # Fallback when no source collection is declared: every <segment> is a
    # wildcard. Weaker than source-bound (cannot detect a missing instance,
    # and will inspect filesystem orphans) but still generic and fail-closed.
    pattern = _re.sub(r'<[^>]+>', '*', str(pattern_text))
    base = pathlib.Path(pattern)
    concrete = str(base) if base.is_absolute() else str(root / base)
    return sorted(pathlib.Path(match) for match in _glob.glob(concrete))

# Parameterized deliverable path (e.g. .../<dataset-id>/<version>/x.json): one
# artifact per runtime-determined instance. Generic across any PRD. Prefer a
# declared source collection; fall back to glob. Zero instances is vacuously
# satisfied unless the contract declares min_instances. required_universe
# contracts keep the single-file path below (their instances are enumerated via
# the registry records loop already present further down).
raw_artifact_path = str(contract.get('artifact_path') or '')
if has_placeholder(raw_artifact_path) and not contract.get('required_universe'):
    art_field = contract.get('instance_artifact_field')
    source_path_value = contract.get('instance_source_path') or contract.get('registry_path')
    records_field = contract.get('instance_source_records_field') or contract.get('registry_records_field')
    if art_field and source_path_value and records_field:
        instance_paths = source_instances(art_field, source_path_value, records_field)
    else:
        instance_paths = glob_instances(raw_artifact_path)
    try:
        min_instances = int(contract.get('min_instances', 0) or 0)
    except (TypeError, ValueError):
        min_instances = 0
    if len(instance_paths) < min_instances:
        failures.append(
            f'declared deliverable requires >= {min_instances} instance(s), '
            f'found {len(instance_paths)} for {raw_artifact_path}'
        )
    # Parameterized deliverables carry the same format rule as single ones: an
    # explicit declaration wins, else infer from the extension. Without this a
    # declared `<run-id>/report.md` instance was parsed as JSON and could never
    # pass — the same unsatisfiable failure as the single-artifact path below.
    declared_format = str(contract.get('artifact_format') or '').strip().lower()
    for instance in instance_paths:
        instance_format = declared_format or (
            'json' if instance.suffix.lower() in ('.json', '.jsonl') else 'text'
        )
        if instance_format != 'json':
            if not instance.is_file() or instance.stat().st_size == 0:
                failures.append(f'declared deliverable instance missing or empty: {instance}')
            continue
        instance_artifact = load_json(instance, f'declared deliverable instance {instance}')
        if instance_artifact is not None:
            internal_consistency(instance_artifact, f'declared deliverable instance {instance}')
    if failures:
        print(json.dumps({'failures': failures}, indent=2))
        raise SystemExit(1)
    print(json.dumps({
        'status': 'declared_deliverable_instances_present',
        'pattern': raw_artifact_path,
        'instance_count': len(instance_paths),
    }))
    raise SystemExit(0)

artifact_path = resolve(contract['artifact_path'])
# Not every deliverable is JSON. A contract may declare a prose or tabular
# artifact (an inventory, a report), and parsing one as JSON fails on line 1
# no matter how good the work is -- a fail-closed gate then demotes correct
# Format of the declared deliverable. An explicit contract declaration always
# wins; otherwise INFER it from the artifact's own extension.
#
# Not every deliverable is JSON — a contract may legitimately declare prose,
# a report, or source text. Parsing one as JSON fails on line 1 however good
# the work is, and a fail-closed gate then demotes it permanently because no
# remediation can make unstructured text parse: every attempt burns with no
# action any agent could take. Defaulting to json had exactly that effect on
# every non-JSON deliverable.
#
# Inference rather than a declared default, because a format nobody remembers
# to declare is a format nobody declares. Structured artifacts keep the full
# predicate suite; textual ones are checked for existence and non-emptiness,
# which is all a contract can honestly assert about unstructured content.
# Domain-neutral: extension only, no knowledge of what the file contains.
artifact_format = str(contract.get('artifact_format') or '').strip().lower()
if not artifact_format:
    artifact_format = 'json' if artifact_path.suffix.lower() in ('.json', '.jsonl') else 'text'
if artifact_format not in ('json', 'text'):
    failures.append(f'unsupported declared artifact_format: {artifact_format}')
    print(json.dumps({'failures': failures}, indent=2))
    raise SystemExit(1)
if artifact_format == 'text':
    if not artifact_path.is_file() or artifact_path.stat().st_size == 0:
        failures.append(f'declared deliverable missing or empty: {artifact_path}')
    if failures:
        print(json.dumps({'failures': failures}, indent=2))
        raise SystemExit(1)
    print(json.dumps({
        'status': 'declared_text_deliverable_present',
        'artifact': str(artifact_path),
        'bytes': artifact_path.stat().st_size,
    }))
    raise SystemExit(0)
artifact = load_json(artifact_path, 'declared deliverable')
if artifact is None:
    print(json.dumps({'failures': failures}, indent=2))
    raise SystemExit(1)
internal_consistency(artifact, 'declared deliverable')

registry = None
if contract.get('registry_path'):
    registry = load_json(resolve(contract['registry_path']), 'declared registry')

data_kind = str(contract.get('data_kind') or '').strip()
payload_declarations = any([
    contract.get('payload_path_field'),
    contract.get('required_fields'),
    contract.get('non_constant_fields'),
    contract.get('series_value_fields'),
])
if data_kind and data_kind != 'record_series':
    failures.append(f'unsupported declared data_kind: {data_kind}')
if payload_declarations and data_kind != 'record_series':
    failures.append('payload substance declarations require data_kind=record_series')
if data_kind == 'record_series':
    if not contract.get('payload_path_field'):
        failures.append('record_series contract has no payload_path_field')
    if not contract.get('required_fields'):
        failures.append('record_series contract has no required_fields')
    if not contract.get('non_constant_fields'):
        failures.append('record_series contract has no non_constant_fields')
    if not contract.get('series_value_fields'):
        failures.append('record_series contract has no series_value_fields')

if not contract.get('required_universe'):
    if failures:
        print(json.dumps({'failures': failures}, indent=2))
        raise SystemExit(1)
    print(json.dumps({'status': 'declared_deliverable_present', 'artifact': str(artifact_path)}))
    raise SystemExit(0)

cells_field = contract.get('cells_field')
cells = get_field(artifact, cells_field, [])
if not isinstance(cells, list):
    failures.append(f'declared cells field is not an array: {cells_field}')
    cells = []
identity_fields = contract.get('cell_identity_fields', [])
if not identity_fields:
    failures.append('required-universe contract has no cell_identity_fields')

universe_fields = contract.get('universe_fields', [])
axes = [get_field(artifact, field, []) for field in universe_fields]
if any(not isinstance(axis, list) or not axis for axis in axes):
    failures.append('declared required universe has an empty or non-array axis')
    required = set()
else:
    required = set(itertools.product(*axes))
indexed = {}
for cell in cells:
    identity = tuple(get_field(cell, field) for field in identity_fields)
    if identity in indexed:
        failures.append(f'duplicate cell identity: {identity}')
    indexed[identity] = cell

if required and set(indexed) != required:
    identity_sort_key = lambda identity: tuple(str(item) for item in identity)
    for identity in sorted(required - set(indexed), key=identity_sort_key):
        failures.append(f'missing required cell: {identity}')
    for identity in sorted(set(indexed) - required, key=identity_sort_key):
        failures.append(f'extra undeclared cell: {identity}')

gaps_field = contract.get('gaps_field')
gaps = get_field(artifact, gaps_field, []) if gaps_field else []
if gaps:
    failures.append(f'declared deliverable contains {len(gaps)} gap record(s)')
gap_identities = {
    tuple(get_field(gap, field) for field in identity_fields)
    for gap in gaps if isinstance(gap, dict)
}

records = get_field(registry, contract.get('registry_records_field'), {}) if registry else {}
if registry is not None and not isinstance(records, dict):
    failures.append('declared registry records field is not an object')
    records = {}

series = []
step_series = []
for identity, cell in sorted(
    indexed.items(),
    key=lambda item: tuple(str(part) for part in item[0]),
):
    label = ':'.join(str(item) for item in identity)
    for field in contract.get('required_true_fields', []):
        if get_field(cell, field) is not True:
            failures.append(f'{label} required true field failed: {field}')
    if identity in gap_identities:
        continue
    for field in contract.get('required_nonempty_fields', []):
        if get_field(cell, field) in (None, '', []):
            failures.append(f'{label} required non-empty field failed: {field}')
    for field in contract.get('positive_count_fields', []):
        try:
            if int(get_field(cell, field, 0) or 0) <= 0:
                failures.append(f'{label} positive count field failed: {field}')
        except (TypeError, ValueError):
            failures.append(f'{label} positive count field is not numeric: {field}')
    for field, minimum in (contract.get('minimum_count_fields') or {}).items():
        try:
            actual = int(get_field(cell, field, 0) or 0)
            if actual < int(minimum):
                failures.append(
                    f'{label} count below declared minimum: {field}={actual} < {minimum}'
                )
        except (TypeError, ValueError):
            failures.append(f'{label} minimum count field is not numeric: {field}')

    key_values = [get_field(cell, field) for field in contract.get('registry_key_fields', [])]
    key = ':'.join(str(value) for value in key_values)
    record = records.get(key) if key else None
    if registry is not None and not isinstance(record, dict):
        failures.append(f'{label} has no declared registry record for key {key}')
        continue
    if record is None:
        record = {}

    for field in contract.get('registry_required_true_fields', []):
        if get_field(record, field) is not True:
            failures.append(f'{label} registry required true field failed: {field}')
    status_field = contract.get('registry_status_field')
    allowed_statuses = {
        normalized(value) for value in contract.get('registry_allowed_statuses', [])
    }
    if status_field and normalized(get_field(record, status_field)) not in allowed_statuses:
        failures.append(f'{label} registry status is not allowed')
    count_field = contract.get('registry_count_field')
    if count_field:
        try:
            registry_count = int(get_field(record, count_field, 0) or 0)
            if registry_count <= 0:
                failures.append(f'{label} registry count is not positive: {count_field}')
            registry_minimum = int(contract.get('registry_minimum_count') or 0)
            if registry_minimum > 0 and registry_count < registry_minimum:
                failures.append(
                    f'{label} registry count below declared minimum: '
                    f'{count_field}={registry_count} < {registry_minimum}'
                )
        except (TypeError, ValueError):
            failures.append(f'{label} registry count is not numeric: {count_field}')
    for cell_field, registry_field in contract.get('registry_identity_fields', {}).items():
        if normalized(get_field(cell, cell_field)) != normalized(get_field(record, registry_field)):
            failures.append(
                f'{label} registry identity mismatch: {cell_field}!={registry_field}'
            )

    validation_path_field = contract.get('validation_path_field')
    if validation_path_field:
        validation_ref = get_field(record, validation_path_field)
        if not validation_ref:
            failures.append(f'{label} registry has no validation path')
        else:
            validation = load_json(resolve(validation_ref), f'{label} validation')
            internal_consistency(validation, f'{label} validation')
            if isinstance(validation, dict):
                overall = normalized(
                    get_field(validation, contract.get('validation_status_field'))
                )
                passed = {
                    normalized(value)
                    for value in contract.get('validation_passed_values', [])
                }
                if passed and overall not in passed:
                    failures.append(f'{label} validation overall status is not passed')

    payload_path_field = contract.get('payload_path_field')
    payload_ref = get_field(record, payload_path_field) if payload_path_field else None
    rows = load_payload(
        resolve(payload_ref) if payload_ref else pathlib.Path(),
        contract.get('payload_format') or 'json',
    ) if payload_path_field else []
    if payload_path_field and not rows:
        failures.append(f'{label} payload has no records')
    for row_index, row in enumerate(rows):
        if not isinstance(row, dict):
            failures.append(f'{label} payload row {row_index} is not an object')
            continue
        for field in contract.get('required_fields', []):
            if get_field(row, field) is None:
                failures.append(f'{label} payload row {row_index} lacks {field}')
    for field in contract.get('non_constant_fields', []):
        values = {
            json.dumps(get_field(row, field), sort_keys=True, separators=(',', ':'))
            for row in rows if isinstance(row, dict)
        }
        if len(values) <= 1:
            failures.append(f'{label} payload field is constant or absent: {field}')
    # Temporal impossibility: an observed series cannot contain records dated in
    # the future. Fabricated payloads are commonly produced by incrementing a
    # timestamp N times from a start date, which overshoots now. Contract-driven
    # and domain-neutral: only runs when the contract names an observation-time
    # field, and only rejects timestamps strictly after the verification instant.
    observed_time_field = contract.get('observed_time_field')
    if observed_time_field and rows:
        now_utc = datetime.datetime.now(datetime.timezone.utc)
        for row_index, row in enumerate(rows):
            if not isinstance(row, dict):
                continue
            raw_value = get_field(row, observed_time_field)
            if raw_value in (None, ''):
                continue
            parsed = parse_timestamp(raw_value)
            if parsed is None:
                failures.append(
                    f'{label} payload row {row_index} {observed_time_field} is not a parseable timestamp: {raw_value}'
                )
                continue
            if parsed > now_utc:
                failures.append(
                    f'{label} payload row {row_index} {observed_time_field} is in the future '
                    f'({raw_value}); an observed series cannot contain future records'
                )
                break

    # Venue-calendar validity. A series observed from a venue that does not
    # trade continuously cannot carry records on days the venue was shut. This
    # is external truth rather than a threshold -- there is no number to tune
    # against, and a generator that emits an evenly spaced series produces
    # closed-day records automatically. Opt-in and domain-neutral: the contract
    # declares which weekdays the venue is closed and any specific closed dates.
    closed_weekdays = contract.get('closed_weekdays')
    closed_dates = {str(value).strip()[:10] for value in contract.get('closed_dates', [])}
    if observed_time_field and rows and (closed_weekdays is not None or closed_dates):
        closed_weekday_set = {int(day) for day in (closed_weekdays or [])}
        closed_hits = []
        for row_index, row in enumerate(rows):
            if not isinstance(row, dict):
                continue
            parsed = parse_timestamp(get_field(row, observed_time_field))
            if parsed is None:
                continue
            if parsed.weekday() in closed_weekday_set:
                closed_hits.append(f'row {row_index} falls on weekday {parsed.weekday()}')
            elif parsed.date().isoformat() in closed_dates:
                closed_hits.append(f'row {row_index} falls on closed date {parsed.date().isoformat()}')
        if closed_hits:
            failures.append(
                f'{label} payload has {len(closed_hits)} record(s) dated when the venue was closed '
                f'({"; ".join(closed_hits[:3])}); an observed series cannot contain them'
            )

    request_path_field = contract.get('request_path_field')
    requested_count_field = contract.get('requested_count_field')
    if request_path_field and requested_count_field:
        request_ref = get_field(record, request_path_field)
        request = load_json(resolve(request_ref), f'{label} request') if request_ref else None
        requested = get_field(request, requested_count_field) if request else None
        try:
            requested_value = int(requested)
            if requested_value <= 0:
                failures.append(f'{label} requested count is not positive: {requested}')
            elif len(rows) < requested_value:
                failures.append(
                    f'{label} delivered count {len(rows)} is below requested count {requested}'
                )
        except (TypeError, ValueError):
            failures.append(f'{label} requested count is not numeric')

    response_path_field = contract.get('response_path_field')
    if response_path_field:
        response_ref = get_field(record, response_path_field)
        if not response_ref:
            failures.append(f'{label} registry has no response path')
        response = load_json(resolve(response_ref), f'{label} response') if response_ref else None
        if isinstance(response, dict):
            for cell_field, response_field in contract.get('response_identity_fields', {}).items():
                if normalized(get_field(cell, cell_field)) != normalized(
                    get_field(response, response_field)
                ):
                    failures.append(
                        f'{label} response identity mismatch: {cell_field}!={response_field}'
                    )

    signature_fields = contract.get('series_value_fields', [])
    if rows and signature_fields:
        tokens = tuple(
            tuple(
                json.dumps(get_field(row, field), sort_keys=True, separators=(',', ':'))
                for field in signature_fields
            )
            for row in rows if isinstance(row, dict)
        )
        digest = hashlib.sha256(
            json.dumps(tokens, separators=(',', ':')).encode()
        ).hexdigest()
        series.append((identity, digest, tokens))
        field_steps = []
        for field in signature_fields:
            try:
                values = [float(get_field(row, field)) for row in rows if isinstance(row, dict)]
            except (TypeError, ValueError):
                continue
            if len(values) < 3:
                continue
            steps = tuple(round(values[index] - values[index - 1], 12) for index in range(1, len(values)))
            # Requiring EVERY step to be identical was trivially evaded: a
            # generated series using two alternating increments has 2 distinct
            # steps and walked straight through. Judge the VARIETY of the step
            # distribution instead. An observed series prices independently each
            # period, so its first differences are near-all distinct; a series
            # whose hundreds of steps take only a handful of values was authored,
            # not observed. Ratio-based, so it does not depend on series length.
            if steps:
                distinct = len(set(steps))
                if distinct == 1:
                    failures.append(f'{label} payload field has a constant first difference: {field}')
                elif len(steps) >= step_variety_min_rows:
                    ratio = distinct / len(steps)
                    if ratio < step_variety_min_ratio:
                        failures.append(
                            f'{label} payload field has only {distinct} distinct first differences '
                            f'across {len(steps)} steps (ratio {ratio:.3f} < {step_variety_min_ratio}): '
                            f'{field} is synthetic, not observed'
                        )
            field_steps.append((field, steps))
        if field_steps:
            step_series.append((identity, tuple(field_steps)))

overlap_rows = int(contract.get('series_overlap_min_rows') or 0)
for index, (identity, digest, tokens) in enumerate(series):
    for other_identity, other_digest, other_tokens in series[:index]:
        if digest == other_digest:
            failures.append(
                f'distinct cells {other_identity} and {identity} have identical payload series'
            )
            continue
        if overlap_rows > 0 and len(tokens) >= overlap_rows and len(other_tokens) >= overlap_rows:
            windows = {
                tokens[offset:offset + overlap_rows]
                for offset in range(len(tokens) - overlap_rows + 1)
            }
            other_windows = {
                other_tokens[offset:offset + overlap_rows]
                for offset in range(len(other_tokens) - overlap_rows + 1)
            }
            if windows.intersection(other_windows):
                failures.append(
                    f'distinct cells {other_identity} and {identity} share a declared payload-series window'
                )

for index, (identity, steps) in enumerate(step_series):
    for other_identity, other_steps in step_series[:index]:
        if steps == other_steps:
            failures.append(
                f'distinct cells {other_identity} and {identity} have identical declared field-step series'
            )

if failures:
    print(json.dumps({'status': 'failed', 'failure_count': len(failures), 'failures': failures}, indent=2))
    raise SystemExit(1)
print(json.dumps({
    'status': 'substantive_deliverable_verified',
    'required_cells': len(required),
    'verified_cells': len(indexed),
    'series_checked': len(series),
}, indent=2))
PY
