from pathlib import Path
import re
import subprocess
p = Path('crates/wt-runtime/src/lib.rs')
assert subprocess.check_output(['git','hash-object',str(p)], text=True).strip() == '63db6842412721e5c825476cc56d9bed20111011', 'runtime base changed'
s = p.read_text()
def change(old, new, count=None):
    global s
    found = s.count(old)
    assert found and (count is None or found == count), (old[:120],found,count)
    s = s.replace(old, new)
change('use tree_sitter::Parser;', 'use tree_sitter::Parser;\n\nmod shared_text;\npub use shared_text::SharedText;\n#[cfg(test)]\nmod performance_tests;',1)
change('pub text: String,', 'pub text: SharedText,',1)
change('    text: String,\n', '    text: SharedText,\n')
change('    groups: HashMap<String, Option<String>>,', '    groups: Arc<HashMap<String, Option<SharedText>>>,',1)
change('    canonical: String,\n    static_string: Option<String>,', '    canonical: SharedText,\n    static_string: Option<SharedText>,',1)
change('    attrs: Vec<(String, AttrValue)>,', '    attrs: Arc<Vec<(String, AttrValue)>>,',1)
change('    Text(String),', '    Text(SharedText),',1)
change('    residual_invocations: usize,', '    residual_invocations: usize,\n    cache_key_bytes_hashed: u64,',1)
change('            "residual_invocations": self.stats.residual_invocations,', '            "residual_invocations": self.stats.residual_invocations,\n            "cache_key_bytes_hashed": self.stats.cache_key_bytes_hashed,',1)
change('digest: digest(file.text.as_bytes()),', 'digest: file.text.digest(&mut self.stats.cache_key_bytes_hashed),')
change('        text: &str,\n        temporary_bytes: &mut usize,', '        text: &SharedText,\n        temporary_bytes: &mut usize,',1)
change('            digest: digest(text.as_bytes()),', '            digest: text.digest(&mut self.stats.cache_key_bytes_hashed),',1)
change('captures.name(name).map(|value| value.as_str().to_owned()),', 'captures.name(name).map(|value| text.slice(value.start()..value.end())),',1)
change('                            .map(|capture| capture.as_str().to_owned()),', '                            .map(|capture| file.text.slice(start + capture.start()..start + capture.end())),',1)
change('                    text: whole.as_str().to_owned(),\n                    groups,', '                    text: file.text.slice(absolute_start..absolute_end),\n                    groups: Arc::new(groups),',1)
change('                text: whole.as_str().to_owned(),\n                groups,', '                text: text.slice(whole.start()..whole.end()),\n                groups: Arc::new(groups),',1)
change('map_or(0, String::len)', 'map_or(0, |value| value.len())')
change('Ok(Value::Text(value))', 'Ok(Value::Text(value.into()))')
change('Ok(Value::Text(path.to_owned()))', 'Ok(Value::Text(path.to_owned().into()))',1)
change('(Value::Attr(attr), "kind") => Ok(Value::Text(attr.kind)),', '(Value::Attr(attr), "kind") => Ok(Value::Text(attr.kind.into())),',1)
change('let value = matched.groups.get(name).and_then(Option::as_deref);', 'let value = matched.groups.get(name).and_then(Option::as_ref);',1)
change('self.charge_temporary(value.map_or(0, str::len))?;', 'self.charge_temporary(value.map_or(0, |value| value.len()))?;',1)
change('Ok(value.map_or(Value::Unit, |value| Value::Text(value.to_owned())))', 'Ok(value.map_or(Value::Unit, |value| Value::Text(value.clone())))',1)
change('text: source[start..end].to_owned(),', 'text: source.slice(start..end),',1)
change('text: source[start..].to_owned(),', 'text: source.slice(start..source.len()),',1)
change('Ok(Value::Text(trimmed.to_owned()))', 'Ok(Value::Text(trimmed.to_owned().into()))',1)
change('Value::Text(part.to_owned())', 'Value::Text(part.to_owned().into())',1)
change('Ok(Value::Text(text.replace(from, to)))', 'Ok(Value::Text(text.replace(from, to).into()))',1)
start=s.index('    fn text_call('); end=s.index('    fn regex_call(',start)
part=s[start:end].replace('let text = expect_text(args.first())?;', 'let text = expect_shared_text(args.first())?;').replace('.map(|value| expect_text(Some(value)))', '.map(|value| expect_shared_text(Some(value)))')
part=part.replace('text_key(&self.files[file.index].path, source, name, &[])', 'text_key(&self.files[file.index].path, source, name, &[], &mut self.arena.stats.cache_key_bytes_hashed)')
part=part.replace('text_key(path, text, name, &argument_texts)', 'text_key(path, text, name, &argument_texts, &mut self.arena.stats.cache_key_bytes_hashed)')
s=s[:start]+part+s[end:]
change('let text = expect_text(args.get(1))?;\n                self.charge_native(text.len())?;', 'let text = expect_shared_text(args.get(1))?;\n                self.charge_native(text.len())?;',1)
change('fn text_key(path: &str, text: &str, operation: &str, arguments: &[&str]) -> TextKey {', 'fn text_key(path: &str, text: &SharedText, operation: &str, arguments: &[&SharedText], bytes_hashed: &mut u64) -> TextKey {',1)
change('        digest: digest(text.as_bytes()),', '        digest: text.digest(bytes_hashed),',1)
change('.map(|argument| digest(argument.as_bytes()))', '.map(|argument| argument.digest(bytes_hashed))',1)
change('fn expect_text(value: Option<&Value>) -> Result<&str> {', 'fn expect_shared_text(value: Option<&Value>) -> Result<&SharedText> {\n    match value {\n        Some(Value::Text(value)) => Ok(value),\n        _ => bail!("expected text"),\n    }\n}\n\nfn expect_text(value: Option<&Value>) -> Result<&str> {',1)
change('canonical: String::new(),', 'canonical: String::new().into(),')
change('canonical: serde_json::to_string(&decoded)?,', 'canonical: serde_json::to_string(&decoded)?.into(),',1)
change('static_string: Some(decoded),', 'static_string: Some(decoded.into()),',1)
change('static_string: serde_json::from_str::<String>(&expression).ok(),', 'static_string: serde_json::from_str::<String>(&expression).ok().map(SharedText::from),',1)
change('canonical: expression,', 'canonical: expression.into(),',1)
s,n=re.subn(r'(result\.push\(InputValue \{[\s\S]*?\n\s*)attrs,',r'\1attrs: Arc::new(attrs),',s)
assert n == 1, ('InputValue constructor',n)
p.write_text(s)
def close_brace(text, start):
    level=1; i=start+1
    while i<len(text):
        if text.startswith('//',i):
            end=text.find('\n',i); i=len(text) if end<0 else end+1; continue
        if text.startswith('/*',i):
            end=text.find('*/',i+2); assert end>=0; i=end+2; continue
        raw=re.match(r'(?:b)?r(#+)?"', text[i:])
        if raw:
            terminator='"'+(raw.group(1) or ''); end=text.find(terminator,i+len(raw.group())); assert end>=0; i=end+len(terminator); continue
        if text[i]=='"':
            i+=1
            while i<len(text):
                if text[i]=='\\': i+=2
                elif text[i]=='"': i+=1; break
                else: i+=1
            continue
        if text[i]=='{': level+=1
        if text[i]=='}':
            level-=1
            if level==0:return i
        i+=1
    raise ValueError('unclosed source literal')
for file in Path('crates').rglob('*.rs'):
    text=file.read_text(); edits=[]
    for m in re.finditer(r'\bSourceFile\s*\{',text):
        if text[max(0,m.start()-10):m.start()].rstrip().endswith('struct'):continue
        end=close_brace(text,m.end()-1); body=text[m.end():end]
        field=re.search(r'\btext\s*:\s*([\s\S]*?)(,\s*)?$',body)
        if field:
            expr=field.group(1).strip()
            if expr.endswith('.into()'):continue
            edits.append((m.end()+field.start(1),m.end()+field.end(1),'('+expr+').into()'))
        else:
            field=re.search(r'\btext\s*,?\s*$',body)
            if field: edits.append((m.end()+field.start(),m.end()+field.end(),'text: text.into(),\n'))
    for start,end,replacement in reversed(edits):text=text[:start]+replacement+text[end:]
    if edits:file.write_text(text)
p=Path('crates/wt-core/src/worker.rs')
s=p.read_text().replace('text: file.text.clone(),','text: file.text.to_string(),')
p.write_text(s)
p=Path('schemas/result.schema.json')
s=p.read_text().replace('regex_evaluations|text_evaluations','regex_evaluations|text_evaluations|cache_key_bytes_hashed')
p.write_text(s)
for name in ['shared_text.rs','performance_tests.rs']:
    Path('crates/wt-runtime/src',name).write_text(Path('.wt-perf-bootstrap',name).read_text())
print('Applied immutable source and shared-payload performance patch.')
