"""Offline, fail-closed validator for the bundled OpenAI catalog and its evidence."""
import copy
import hashlib
import json
from pathlib import Path
import re
import sys
from jsonschema import Draft202012Validator, FormatChecker

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    'gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna', 'gpt-6-sol',
    'gpt-6-astra', 'gpt-6.1-sol', 'gpt-6-luna', 'gpt-5.5', 'gpt-5.5-pro',
}


def keys(obj, expected):
    if not isinstance(obj, dict) or set(obj) != set(expected.split()):
        raise ValueError(f'Unknown or missing fields: expected {expected}, got {obj}')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def validate_legacy(catalog, sources):
    keys(catalog, 'schema_version kind revision activation coverage models')
    require(catalog['schema_version'] == 1 and catalog['revision'] == 1, 'version')
    require(catalog['kind'] == 'timem.provider_model_catalog', 'kind')
    require(catalog['activation'] == 'pending_runtime_integration', 'not runtime-ready')
    require(catalog['coverage'] == 'specified_model_pages_only', 'coverage')
    keys(sources, 'schema_version sources')
    require(sources['schema_version'] == 1, 'sources version')
    require(set(sources['sources']) == EXPECTED, 'source coverage')
    require(len(catalog['models']) == 9, 'model count')
    require({m['model_id'] for m in catalog['models']} == EXPECTED, 'model identities')
    for model in catalog['models']:
        keys(model, 'id revision provider_id model_id label connection dimensions profiles documented_facts')
        mid = model['model_id']
        source = sources['sources'][mid]
        keys(source, 'url retrieved_url retrieved_at sha256 markdown')
        text = source['markdown']
        require(source['url'] == f'https://developers.openai.com/api/docs/models/{mid}', 'source URL')
        require(source['retrieved_url'] == source['url'] + '.md', 'retrieval URL')
        require(hashlib.sha256(text.encode()).hexdigest() == source['sha256'], 'snapshot hash')
        require(f'Model ID: `{mid}`' in text, 'source model mismatch')
        require(model['id'] == 'openai/' + mid and model['provider_id'] == 'openai', 'pair identity')
        require(model['revision'] == 1 and model['label'] == text.splitlines()[0][2:], 'identity metadata')
        require(model['connection'] == {
            'default_base_url': 'https://api.openai.com/v1',
            'allow_base_url_override': True, 'auth_handler': 'bearer_api_key',
        }, 'connection contract')
        require(len(model['dimensions']) == 1, 'unverified dimension added')
        dim = model['dimensions'][0]
        keys(dim, 'id label type allow_unset initial_selection service_default options source_refs')
        require(dim['id'] == 'reasoning.effort' and dim['type'] == 'enum', 'dimension handler')
        require(dim['allow_unset'] is True and dim['initial_selection'] == {'state': 'unset'}, 'unset semantics')
        sentence = re.search(r'[Rr]easoning\.effort supports:?\s*(.*?)\.', text.replace('`', ''), re.S).group(1)
        options = re.findall(r'\b(none|low|medium|high|xhigh|max|minimal)\b', sentence)
        for option in dim['options']:
            keys(option, 'id label')
            require(isinstance(option['label'], str) and option['label'], 'option label')
        require([o['id'] for o in dim['options']] == options, 'options differ from source')
        default = re.search(r'(\w+) \(default\)', sentence)
        require(dim['service_default'] == (default.group(1) if default else None), 'invented default')
        require(dim['source_refs'] == [mid], 'dimension evidence')
        protocols = []
        for label, protocol, route in [
            ('Responses', 'openai-responses', '/responses'),
            ('Chat Completions', 'openai-compatible', '/chat/completions'),
        ]:
            if not re.search(r'\| ' + re.escape(label) + r' \| [^\n]+ \| Supported \|', text):
                continue
            protocols.append(protocol)
            profile = next(p for p in model['profiles'] if p['protocol'] == protocol)
            keys(profile, 'protocol route dimension_ids function_calling source_refs')
            require(profile['route'] == route and profile['source_refs'] == [mid], 'protocol route/evidence')
            require(profile['dimension_ids'] == ['reasoning.effort'], 'unresolved dimension')
            expected = {'support': 'supported', 'require_selection': None}
            if protocol == 'openai-compatible' and mid in {'gpt-6-sol', 'gpt-6-luna'}:
                require('only with `reasoning_effort` set to `none`' in text, 'conditional tool evidence')
                expected = {'support': 'conditional', 'require_selection': {'dimension': 'reasoning.effort', 'equals': 'none'}}
            if protocol == 'openai-compatible' and mid == 'gpt-6.1-sol':
                require('Chat Completions is supported without\ntool calling' in text, 'no tools evidence')
                expected = {'support': 'unsupported', 'require_selection': None}
            require(profile['function_calling'] == expected, 'tool capability restriction')
        require([p['protocol'] for p in model['profiles']] == protocols, 'protocol whitelist')
        facts = model['documented_facts']
        keys(facts, 'source_refs input_modalities output_modalities context_window_tokens max_input_tokens max_output_tokens streaming default_snapshot')
        require(facts['source_refs'] == [mid], 'facts evidence')
        for field, pattern in [('context_window_tokens', r'- ([\d,]+) context window'),
                               ('max_input_tokens', r'- Maximum input tokens: ([\d,]+)'),
                               ('max_output_tokens', r'- ([\d,]+) max output tokens')]:
            match = re.search(pattern, text)
            require(facts[field] == (int(match.group(1).replace(',', '')) if match else None), 'invented limit: ' + field)
        for direction in ['input', 'output']:
            value = re.search(r'- ' + direction.capitalize() + r' modalities: (.+)', text).group(1).split(', ')
            require(facts[direction + '_modalities'] == value, 'modality evidence')
        features = text.split('## Supported features\n', 1)[1].split('\n## ', 1)[0]
        require(facts['streaming'] == ('supported' if '- streaming\n' in features else 'unknown'), 'stream evidence')
        require(facts['default_snapshot'] == re.search(r'- Default snapshot: `([^`]+)`', text).group(1), 'snapshot evidence')


def validate(catalog, sources):
    schema = json.loads((ROOT / 'resources/provider_model_catalog.schema.json').read_text())
    Draft202012Validator.check_schema(schema)
    errors = list(Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(catalog))
    require(not errors, str(errors[0]) if errors else '')
    legacy = copy.deepcopy(catalog)
    legacy.pop('$schema')
    legacy['schema_version'] = legacy['revision'] = 1
    legacy['activation'] = 'pending_runtime_integration' # v1 evidence projection only
    for model in legacy['models']:
        model['revision'] = 1
        dims = model['dimensions']
        require(len(dims) == 2 and [d['id'] for d in dims] == ['api.protocol', 'reasoning.effort'], 'dimension identities')
        protocol, effort = dims
        values = [o['id'] for o in effort['options']]
        require(len(set(values)) == len(values), 'duplicate option')
        require(protocol['handler'] == 'api_protocol' and effort['handler'] == 'reasoning_effort', 'handler mismatch')
        require(protocol['ordered'] is False and protocol['allow_unset'] is False, 'protocol semantics')
        require(effort['ordered'] is True and effort['allow_unset'] is True, 'effort semantics')
        require(protocol['source_refs'] == [model['model_id']], 'protocol source')
        require([o['id'] for o in protocol['options']] == [p['protocol'] for p in model['profiles']], 'protocol options')
        require(protocol['default_policy'] == {'type':'explicit','value':'openai-responses','source':'product'}, 'protocol default')
        policy = effort['default_policy']
        require('official_value' in policy, 'effort default type')
        candidates = policy['fallback']['candidates']
        require(candidates == [v for v in values if v != 'none'], 'middle candidates')
        require(policy['official_value'] is None or policy['official_value'] in values, 'default outside options')
        default = policy['official_value'] or candidates[(len(candidates)-1)//2]
        if model['model_id'] == 'gpt-6-astra':
            require(default == 'high', 'Astra middle default')
        expected_rules = []
        for profile in model['profiles']:
            is_chat = profile['protocol'] == 'openai-compatible'
            require(profile['bindings'] == [{'dimension':'reasoning.effort','handler':'enum_body_field','path':'/reasoning_effort' if is_chat else '/reasoning/effort'}], 'binding mismatch')
            support = profile['function_calling']
            if support == 'conditional':
                expected_rules.append({'id':'chat-native-tools-require-none','when':{'all':[{'dimension':'api.protocol','equals':'openai-compatible'},{'context':'tool_mode','equals':'native_function_calling'}]},'effects':[{'type':'fixed_selection','dimension':'reasoning.effort','value':'none','send_policy':'explicit'}],'reason':'此模型通过 Chat Completions 使用原生函数调用时必须关闭推理。若要开启推理，请切换到 Responses。','source_refs':[model['model_id']]})
                require('none' in values, 'fixed value outside options')
            if support == 'unsupported':
                expected_rules.append({'id':'native-tools-disable-chat','when':{'all':[{'context':'tool_mode','equals':'native_function_calling'}]},'effects':[{'type':'disable_option','dimension':'api.protocol','value':'openai-compatible'}],'reason':'此模型的 Chat Completions 不支持原生函数调用，请使用 Responses。','source_refs':[model['model_id']]})
            profile['function_calling'] = {'support': support, 'require_selection': {'dimension':'reasoning.effort','equals':'none'} if support == 'conditional' else None}
            profile.pop('bindings')
            profile.pop('binding_status')
        require(model.pop('constraints') == expected_rules, 'missing or conflicting constraints')
        limits = model.pop('limits')
        require(limits.pop('source_refs') == [model['model_id']], 'limits evidence')
        model['documented_facts'].update(limits)
        effort['service_default'] = policy['official_value']
        effort['initial_selection'] = {'state':'unset'}
        for key in ['default_policy','ordered','handler','ui']:
            effort.pop(key)
        model['dimensions'] = [effort]
    # Reuse independent source-evidence checks from v1; this projection is validation-only,
    # never a loader or migration path for the application.
    validate_legacy(legacy, sources)


def main():
    catalog = json.loads((ROOT / 'resources/openai_model_catalog.json').read_text())
    sources = json.loads((ROOT / 'resources/openai_model_catalog.sources.json').read_text())
    validate(catalog, sources)
    print('PASS: Draft 2020-12 schema + 9 model evidence/default/constraint/binding checks')
    if '--self-test' in sys.argv:
        def unknown(c): c['models'][0]['dimensions'][1]['magic'] = True
        def option(c): c['models'][0]['dimensions'][1]['options'].append({'id':'minimal','label':'Minimal'})
        def default(c): c['models'][4]['dimensions'][1]['default_policy']['official_value'] = 'medium'
        def restriction(c): c['models'][3]['constraints'] = []
        def limit(c): c['models'][7]['limits']['max_input_tokens'] = 922000
        def activation(c): c['activation'] = 'active'
        def binding(c): c['models'][0]['profiles'][0]['bindings'][0]['path'] = '/reasoning_effort'
        def handler(c): c['models'][0]['dimensions'][1]['handler'] = 'magic'
        def middle(c): c['models'][4]['dimensions'][1]['default_policy']['fallback']['candidates'] = ['low']
        def fixed(c): c['models'][3]['constraints'][0]['effects'][0]['value'] = 'high'
        def duplicate(c): c['models'][3]['constraints'] *= 2
        for mutate in [unknown,option,default,restriction,limit,activation,binding,handler,middle,fixed,duplicate]:
            invalid = copy.deepcopy(catalog)
            mutate(invalid)
            try:
                validate(invalid, sources)
            except ValueError:
                print('PASS: rejects', mutate.__name__)
            else:
                raise AssertionError('Accepted invalid catalog: ' + mutate.__name__)


if __name__ == '__main__':
    main()
