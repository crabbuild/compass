"""Additional source-authored broad behaviors and exact/directed controls.

Questions are selected from pinned source, never from either tool's output.
Exact lookup controls intentionally name declarations; they are not broad-query
paraphrases. Selection rationales identify distinct runtime responsibilities.
"""
ANCHORS = {}
TASKS = []
REPO = None


def repo(name):
    global REPO
    REPO = name


def a(key, file, symbol, owner, declaration, *witnesses, occurrence=0):
    ANCHORS[REPO,key] = dict(file=file,symbol=symbol,owner=owner,
        declaration=declaration,occurrence=occurrence,
        witnesses=[dict(needle=n,statement=s) for n,s in witnesses])


def py(key,file,owner,symbol,*witnesses):
    ANCHORS[REPO,key]=dict(file=file,symbol=symbol,owner=owner,
        pythonSelector=True,witnesses=[dict(needle=n,statement=s) for n,s in witnesses])


def task(kind,key,question,anchors,*,rationale,edges=()):
    TASKS.append(dict(id=f'{REPO}.expansion-{key}',repository=REPO,
        cluster=f'{REPO}.expansion-{key}',kind=kind,question=question,
        anchors=list(anchors),edges=list(edges),rationale=None,
        selectionRationale=rationale))


def b(key,question,*anchors,rationale):
    task('broad',key,question,anchors,rationale=rationale)


def e(key,anchor,rationale):
    value=ANCHORS[REPO,anchor]
    name=(value['owner']+'.' if value['owner'] else '')+value['symbol']
    task('exact',key,f'Find {name} in {value["file"]}.',[anchor],rationale=rationale)


def d(key,question,source,target,call,rationale):
    task('directed',key,question,[source,target],edges=[(source,target,call)],rationale=rationale)


# Fourteen additional responsibilities balance the initial broad panel at ten
# conservative source-task clusters per repository. Original160 is unchanged.
repo('itsdangerous')
a('signed-object','src/itsdangerous/serializer.py','dumps','Serializer','    def dumps(self, obj:',
  ('payload = want_bytes(self.dump_payload(obj))','Object serialization produces the payload bytes.'),
  ('rv = self.make_signer(salt).sign(payload)','The serialized payload is signed before returning.'))
a('signer-policy','src/itsdangerous/serializer.py','make_signer','Serializer','    def make_signer(',
  ('salt = self.salt','Absent per-call salt uses the serializer salt.'),
  ('return self.signer(self.secret_keys, salt=salt, **self.signer_kwargs)','The configured signer is constructed with all rotation keys and signer options.'))
b('signed-object','Where is an object serialized and signed before producing its output string?', 'signed-object',rationale='Serialization-and-signing orchestration is distinct from key derivation, signature comparison, decoding and compression.')
b('signer-policy','Where is a serializer-specific signer constructed with its salt, key rotation list and options?', 'signer-policy',rationale='Construction of signing policy is a separate runtime declaration from signature generation and payload formatting.')

repo('marshmallow')
a('validate-only','src/marshmallow/schema.py','validate','Schema','    def validate(',
  ('self._do_load(data, many=many, partial=partial, postprocess=False)','Validation disables postprocessing.'),
  ('return typing.cast("dict[str, list[str]]", exc.messages)','Validation returns error messages instead of propagating the validation exception.'))
a('nested-cache','src/marshmallow/fields.py','schema','Nested','    def schema(self)',
  ('if not self._schema:','The nested schema is initialized lazily.'),
  ('return self._schema','The cached nested schema is returned on subsequent access.'))
a('field-options','src/marshmallow/schema.py','_init_fields','Schema','    def _init_fields(',
  ('field_names = field_names - self.exclude','Excluded fields are removed during field setup.'),
  ('if not field_obj.dump_only:','Dump-only fields are kept out of the load field map.'),
  ('if not field_obj.load_only:','Load-only fields are kept out of the dump field map.'))
b('validate-only','Where can schema validation return an error dictionary without running post-load processing?', 'validate-only',rationale='Validation-only error conversion is a different public contract from loading or dumping.')
b('nested-cache','Where is the nested schema object initialized lazily and reused?', 'nested-cache',rationale='Nested schema lifecycle and caching are separate from individual field serialization or missing-value checks.')
b('field-options','Where are excluded fields and load-only or dump-only field maps prepared?', 'field-options',rationale='Option-driven field partitioning occurs before value serialization and is a distinct setup responsibility.')

repo('httprouter')
a('router-defaults','router.go','New','','func New(',
  ('RedirectTrailingSlash:  true,','New routers enable trailing-slash redirection.'),
  ('HandleMethodNotAllowed: true,','New routers enable method-not-allowed handling.'))
b('router-defaults','Where are a new router\'s redirect and method-handling defaults enabled?', 'router-defaults',rationale='Initial router configuration is distinct from runtime request dispatch and redirect status selection.')

repo('go-cache')
a('flush','cache.go','Flush','cache','func (c *cache) Flush(',
  ('c.items = map[string]Item{}','Flushing replaces the complete item map while holding the cache lock.'))
b('flush','Where are all cached entries discarded at once?', 'flush',rationale='Whole-cache invalidation is separate from TTL expiry, reads, persistence and individual mutation.')

repo('humantime')
a('smart-format','src/date.rs','fmt','Rfc3339Timestamp','    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result',
  ('let offset = if self.1 == Seconds || nanos == 0 && self.1 == Smart {','Smart timestamp formatting omits a zero fractional part.'),
  ("buf[19] = b'Z';",'The whole-second result terminates directly with the UTC marker.'))
a('first-digit','src/duration.rs','parse_first_char','Parser','    fn parse_first_char(',
  ('c if c.is_whitespace() => continue,','Leading whitespace is skipped before a duration number.'),
  ('return Err(Error::NumberExpected(off));','An unexpected nonnumeric initial character reports NumberExpected.'))
a('unit-error','src/duration.rs','fmt','Error','    fn fmt(&self, f: &mut fmt::Formatter<',
  ('supported units: ns, us/µs, ms, sec, min, hours, days,','Nonempty unknown-unit error messages list supported duration units.'))
a('borrow-asref','src/wrapper.rs','as_ref','Duration','    fn as_ref(&self) -> &StdDuration',
  ('&self.0','AsRef borrows the stored standard duration.'))
a('borrow-deref','src/wrapper.rs','deref','Duration','    fn deref(&self) -> &StdDuration',
  ('&self.0','Deref borrows the same stored duration.'))
b('smart-format','Where does timestamp display omit fractional digits for whole seconds in smart mode?', 'smart-format',rationale='Timestamp presentation is distinct from strict/weak date parsing and duration formatting.')
b('first-digit','Where are leading duration whitespace and a missing initial number handled?', 'first-digit',rationale='Initial token recognition is separate from unit conversion, decimal parsing and numeric carry.')
b('unit-error','Where does a nonempty unknown duration unit produce a message listing supported units?', 'unit-error',rationale='Human-readable diagnostic construction is distinct from parsing the unit and is counted once despite the existing ambiguity control.')
b('borrow','Where do the duration wrapper\'s borrowing traits expose a reference to the standard duration?', 'borrow-asref','borrow-deref',rationale='Borrowing an already parsed duration is an ownership/API responsibility, separate from parsing and rendering. The two trait implementations form one cluster.')

repo('p-queue')
a('timeout-setting','source/index.ts','timeout','PQueue','\tset timeout(',
  ('validateTimeout(timeout);','A changed queue timeout is validated immediately.'),
  ('this.#timeout = timeout;','Only the validated timeout is stored.'))
b('timeout-setting','Where is a queue timeout setting validated before it is changed?', 'timeout-setting',rationale='Runtime timeout configuration validation is separate from concurrency/rate admission, queue ordering and abort listener management.')

repo('superjson')
a('dual-map','src/double-indexed-kv.ts','set','DoubleIndexedKV','  set(key:',
  ('this.keyToValue.set(key, value);','The forward identifier-to-value map is updated.'),
  ('this.valueToKey.set(value, key);','The reverse value-to-identifier map is updated.'))
a('representative','src/plainer.ts','generateReferentialEqualityAnnotations','','export function generateReferentialEqualityAnnotations(',
  ('if (!dedupe) {','Representative reordering is allowed only when not deduplicating.'),
  ('.sort((a, b) => a.length - b.length);','The shortest reference path is preferred when reordering is safe.'))
b('dual-map','Where are forward and reverse registry mappings updated together?', 'dual-map',rationale='Bidirectional state updates are distinct from reverse lookup and clearing.')
b('representative','Where is a shorter representative path selected for repeated object references when deduplication is off?', 'representative',rationale='Choosing a reference representative is distinct from discovering cycles, serializing values and restoring reference aliases.')

repo('requests')
for key,file,owner,symbol,rationale in [
 ('header-encoding','utils.py','','get_encoding_from_headers','Response character encoding is inferred from Content-Type, independent of streaming, redirects or authentication.'),
 ('cookie-import','cookies.py','','cookiejar_from_dict','Dictionary cookie import handles overwrite policy and jar construction.'),
 ('redirect','sessions.py','SessionRedirectMixin','resolve_redirects','Redirect iteration rebuilds requests and response history.'),
 ('transport','adapters.py','HTTPAdapter','send','Adapter dispatch handles transport exceptions and socket timeout policy.'),
 ('status-error','models.py','Response','raise_for_status','HTTP status error conversion is distinct from byte streaming.'),
 ('header-map','structures.py','CaseInsensitiveDict','__setitem__','Case-insensitive key storage preserves original casing.'),
 ('basic-auth','auth.py','HTTPBasicAuth','__call__','Basic authorization construction is a distinct authentication path.'),
 ('hook','hooks.py','','dispatch_hook','Hook dispatch permits non-None replacement of hook data.'),
 ('pickle','models.py','Response','__getstate__','Response serialization forces content consumption before state extraction.')]:
 py(key,'src/requests/'+file,owner,symbol)
 e(key,key,rationale)
py('api-get','src/requests/api.py','','get')
py('api-request','src/requests/api.py','','request')
d('api-call','Which shared request function does the GET convenience function call?', 'api-get','api-request','return request("get", url, params=params, **kwargs)','A public verb convenience delegates to general session dispatch; endpoints are excluded from exact controls.')
py('lines','src/requests/models.py','Response','iter_lines')
py('content','src/requests/models.py','Response','iter_content')
d('stream-call','Which chunk iterator is called by Response.iter_lines?', 'lines','content','for chunk in self.iter_content(','Line streaming delegates to byte/chunk streaming; no response-stream endpoint is reused by another control.')
py('env','src/requests/sessions.py','Session','merge_environment_settings')
py('proxy','src/requests/utils.py','','get_environ_proxies')
d('environment-call','Which environment proxy resolver does Session.merge_environment_settings call?', 'env','proxy','env_proxies = get_environ_proxies(url, no_proxy=no_proxy)','Environment merge uses an imported package function; this is distinct from HTTP request dispatch and transport handling.')

repo('urllib3')
for key,file,owner,symbol,rationale in [
 ('retry','util/retry.py','Retry','get_backoff_time','Retry history determines exponential backoff.'),
 ('timeout','util/timeout.py','Timeout','read_timeout','Read timeout accounts for the connection time already consumed.'),
 ('multipart','filepost.py','','encode_multipart_formdata','Multipart encoding constructs boundaries and per-field payloads.'),
 ('tls-context','util/ssl_.py','','create_urllib3_context','TLS context configuration sets protocol/cipher/certificate policy.'),
 ('url','util/url.py','','parse_url','URL parsing produces the normalized structured URL components.'),
 ('body','response.py','HTTPResponse','read','Response reading handles content decoding and remaining-length bookkeeping.'),
 ('header-values','_collections.py','HTTPHeaderDict','add','Header addition preserves duplicate header values.'),
 ('pool','poolmanager.py','PoolManager','connection_from_context','Connection-pool selection resolves a pool key from request context.'),
 ('tunnel','util/proxy.py','','connection_requires_http_tunnel','Proxy tunneling policy distinguishes schemes and forwarding options.')]:
 py(key,'src/urllib3/'+file,owner,symbol);e(key,key,rationale)
py('normalize-many','src/urllib3/connection.py','','_normalize_header_values')
py('normalize-one','src/urllib3/connection.py','','_normalize_header_value')
d('header-call','Which single-value normalizer is called by _normalize_header_values?', 'normalize-many','normalize-one','normalized_value = _normalize_header_value(value)','Obsolete folded header normalization is distinct from duplicate-value dictionary storage.')
py('connect','src/urllib3/util/connection.py','','create_connection')
py('family','src/urllib3/util/connection.py','','allowed_gai_family')
d('family-call','Which address-family selector is called while creating a socket connection?', 'connect','family','family = allowed_gai_family()','Socket address-family selection is a distinct low-level network boundary.')
py('unwrap','src/urllib3/util/ssltransport.py','SSLTransport','unwrap')
py('io-loop','src/urllib3/util/ssltransport.py','SSLTransport','_ssl_io_loop')
d('unwrap-call','Which I/O loop does SSLTransport.unwrap call?', 'unwrap','io-loop','self._ssl_io_loop(self.sslobj.unwrap)','TLS transport unwrapping delegates through an explicit concrete receiver; it is separate from TLS context configuration.')

repo('pluggy')
for key,file,owner,symbol,rationale in [
 ('spec-marker','_decorators.py','HookspecMarker','__call__','Hook specification decoration stores specification options.'),
 ('register-plugin','_manager.py','PluginManager','register','Plugin registration discovers and attaches hook implementations.'),
 ('pending','_manager.py','PluginManager','check_pending','Pending hook implementations are checked for unknown specifications.'),
 ('entrypoints','_manager.py','PluginManager','load_setuptools_entrypoints','Installed package entrypoints are loaded and distribution information retained.'),
 ('forced-error','_result.py','Result','force_exception','Result state can be replaced by a chosen exception.'),
 ('history','_caller.py','HookCaller','_maybe_apply_history','Historic hook calls can be replayed for a newly added implementation.'),
 ('config','_config.py','','normalize_hookimpl_opts','Hook implementation options receive explicit default values.'),
 ('arguments','_decorators.py','','varnames','Function argument introspection separates positional and keyword-only parameters.'),
 ('trace-processor','_tracing.py','TagTracer','setprocessor','A trace processor is registered for a tag key.')]:
 py(key,'src/pluggy/'+file,owner,symbol);e(key,key,rationale)
py('has','src/pluggy/_manager.py','PluginManager','has_plugin')
py('get','src/pluggy/_manager.py','PluginManager','get_plugin')
d('lookup-call','Which lookup does PluginManager.has_plugin delegate to?', 'has','get','return self.get_plugin(name) is not None','Membership lookup is a separate concrete query from plugin registration.')
py('trace-sub','src/pluggy/_tracing.py','TagTracerSub','__call__')
py('trace-root','src/pluggy/_tracing.py','TagTracer','_processmessage')
d('trace-call','Which root tracer method does TagTracerSub.__call__ invoke?', 'trace-sub','trace-root','self.root._processmessage(self.tags, args)','Sub-tracer emission routes through the root tracer; constructor typing/binding is source reviewed.')
py('repr','src/pluggy/_tracing.py','','_safe_repr')
py('repr-error','src/pluggy/_tracing.py','','_describe_failure')
d('repr-call','Which failure-description helper handles exceptions in _safe_repr?', 'repr','repr-error','text = _describe_failure(exc, obj, "repr")','Safe representation handles hostile repr failures independently of tracer event emission.')

repo('attrs')
for key,file,owner,symbol,rationale in [
 ('dict','_funcs.py','','asdict','Attribute instances are converted to dictionaries with recursive/filter policies.'),
 ('evolve','_make.py','','evolve','An instance is evolved by rebuilding its class with changed attribute values.'),
 ('generic','_compat.py','','get_generic_base','Generic aliases expose their underlying origin for compatibility handling.'),
 ('slots','_make.py','_ClassBuilder','_create_slots_class','Slotted class construction preserves layout and inherited attributes.'),
 ('factory-state','_make.py','Factory','__getstate__','Factory callables and takes_self state are serialized explicitly.'),
 ('converter-code','_make.py','Converter','_fmt_converter_call','Converter call source includes configured self/field argument requirements.'),
 ('version','_version_info.py','VersionInfo','_from_version_string','Version strings are parsed into ordered version metadata.'),
 ('bool','converters.py','','to_bool','Boolean conversion recognizes explicit string/integer truth values.'),
 ('deep-validation','validators.py','_DeepIterable','__call__','Deep iterable validation checks the container and its members.')]:
 py(key,'src/attr/'+file,owner,symbol);e(key,key,rationale)
py('filter','src/attr/filters.py','','include')
py('filter-split','src/attr/filters.py','','_split_what')
d('filter-call','Which selector classifier does the include filter call?', 'filter','filter-split','cls, names, attrs = _split_what(what)','Filter construction classifies selectors before building its predicate.')
py('hash','src/attr/_make.py','','_add_hash')
py('hash-script','src/attr/_make.py','','_make_hash_script')
d('hash-call','Which script generator is called by _add_hash?', 'hash','hash-script','script, globs = _make_hash_script(','Hash implementation generation is distinct from class layout and converter invocation formatting.')
py('disable','src/attr/validators.py','','set_disabled')
py('validator-config','src/attr/_config.py','','set_run_validators')
d('validator-call','Which validator configuration setter does set_disabled call?', 'disable','validator-config','set_run_validators(not disabled)','Global validator configuration is separate from a particular deep-container validator.')

repo('viper')
for key,symbol,owner,decl,rationale in [
 ('new','New','','func New(','Construction initializes independent config layers and case-insensitive maps.'),
 ('watch','WatchConfig','Viper','func (v *Viper) WatchConfig(','Filesystem watch handles create/write/remove/rename events.'),
 ('environment','BindEnv','Viper','func (v *Viper) BindEnv(','Environment bindings record candidate variable names.'),
 ('alias','RegisterAlias','Viper','func (v *Viper) RegisterAlias(','Alias registration controls configuration-key indirection.'),
 ('write','WriteConfigAs','Viper','func (v *Viper) WriteConfigAs(','Configuration persistence selects an explicit output filename.'),
 ('decode','Unmarshal','Viper','func (v *Viper) Unmarshal(','Configuration values are decoded into a user-supplied value.'),
 ('settings','AllSettings','Viper','func (v *Viper) AllSettings(','All settings are assembled into a nested map.'),
 ('default','SetDefault','Viper','func (v *Viper) SetDefault(','Fallback values are stored in the defaults layer.'),
 ('origin','InConfig','Viper','func (v *Viper) InConfig(','The API checks membership in the file-backed configuration layer.')]:
 a(key,'viper.go',symbol,owner,decl);e(key,key,rationale)
a('integer','viper.go','GetInt','Viper','func (v *Viper) GetInt(')
a('get','viper.go','Get','Viper','func (v *Viper) Get(')
d('integer-call','Which generic value lookup does Viper.GetInt call?', 'integer','get','return cast.ToInt(v.Get(key))','Typed integer retrieval delegates to the concrete receiver lookup; neither endpoint is used by another control.')
a('merge','viper.go','MergeConfig','Viper','func (v *Viper) MergeConfig(')
a('reader','viper.go','unmarshalReader','Viper','func (v *Viper) unmarshalReader(')
d('reader-call','Which reader decoder does Viper.MergeConfig call?', 'merge','reader','if err := v.unmarshalReader(in, config); err != nil {','Reader-based merge decodes bytes before merging configuration data.')
a('flags','viper.go','BindFlagValues','Viper','func (v *Viper) BindFlagValues(')
a('flag','viper.go','BindFlagValue','Viper','func (v *Viper) BindFlagValue(')
d('flags-call','Which per-flag binder is called by Viper.BindFlagValues?', 'flags','flag','if err = v.BindFlagValue(flag.Name(), flag); err != nil {','Flag-set integration loops over supplied flags and binds each to its long name.')

repo('pflag')
for key,file,symbol,owner,decl,rationale in [
 ('normalize','flag.go','SetNormalizeFunc','FlagSet','func (f *FlagSet) SetNormalizeFunc(','Installing a normalization function rebuilds name indexes.'),
 ('deprecate','flag.go','MarkDeprecated','FlagSet','func (f *FlagSet) MarkDeprecated(','Flag deprecation validates and records a warning message.'),
 ('register','flag.go','AddFlag','FlagSet','func (f *FlagSet) AddFlag(','Flag registration rejects duplicate long names and shorthand conflicts.'),
 ('shorthand','flag.go','ShorthandLookup','FlagSet','func (f *FlagSet) ShorthandLookup(','Shorthand lookup validates a single-character key.'),
 ('visit','flag.go','Visit','FlagSet','func (f *FlagSet) Visit(','Only explicitly set flags are visited in the configured order.'),
 ('output','flag.go','SetOutput','FlagSet','func (f *FlagSet) SetOutput(','Output routing stores a caller-supplied writer.'),
 ('annotation','flag.go','SetAnnotation','FlagSet','func (f *FlagSet) SetAnnotation(','Application-specific flag annotations are stored by key.'),
 ('ip','ip.go','Set','ipValue','func (i *ipValue) Set(','IP-address input uses net.ParseIP and rejects invalid values.'),
 ('dash','flag.go','ArgsLenAtDash','FlagSet','func (f *FlagSet) ArgsLenAtDash(','The position of the argument terminator is exposed without returning flag state.')]:
 a(key,file,symbol,owner,decl);e(key,key,rationale)
a('parse','flag.go','Parse','FlagSet','func (f *FlagSet) Parse(')
a('args','flag.go','parseArgs','FlagSet','func (f *FlagSet) parseArgs(')
d('parse-call','Which argument loop is called by FlagSet.Parse?', 'parse','args','err := f.parseArgs(arguments, set)','Top-level parsing routes through the concrete internal argument loop.')
a('defaults','flag.go','PrintDefaults','FlagSet','func (f *FlagSet) PrintDefaults(')
a('usages','flag.go','FlagUsages','FlagSet','func (f *FlagSet) FlagUsages(')
d('help-call','Which usage formatter does FlagSet.PrintDefaults call?', 'defaults','usages','usages := f.FlagUsages()','Help rendering is independent of argument parsing and typed value lookup.')
a('duration','duration.go','GetDuration','FlagSet','func (f *FlagSet) GetDuration(')
a('typed','flag.go','getFlagType','FlagSet','func (f *FlagSet) getFlagType(')
d('duration-call','Which generic typed accessor does FlagSet.GetDuration call?', 'duration','typed','f.getFlagType(name, "duration", durationConv)','Typed duration access validates the registered value type before converting it.')

repo('jwt')
for key,file,symbol,owner,decl,rationale in [
 ('hmac','hmac.go','Sign','SigningMethodHMAC','func (m *SigningMethodHMAC) Sign(','HMAC signing enforces key type and hash availability.'),
 ('rsa-key','rsa_utils.go','ParseRSAPublicKeyFromPEM','','func ParseRSAPublicKeyFromPEM(','RSA public keys are decoded from supported PEM encodings; no secret values are executed or copied.'),
 ('parser','parser.go','NewParser','','func NewParser(','Parser construction applies options to its validator and defaults.'),
 ('segment','parser.go','DecodeSegment','Parser','func (p *Parser) DecodeSegment(','JWT segment decoding applies parser-specific padding and strict-base64 policy.'),
 ('claims','validator.go','Validate','Validator','func (v *Validator) Validate(','Claim-policy validation aggregates timestamp/audience/issuer failures.'),
 ('audience-json','types.go','UnmarshalJSON','ClaimStrings','func (s *ClaimStrings) UnmarshalJSON(','Audience JSON accepts its documented scalar/array/null shapes.'),
 ('algorithms','signing_method.go','GetAlgorithms','','func GetAlgorithms(','The signing algorithm registry is enumerated under a read lock.'),
 ('errors','errors.go','joinErrors','','func joinErrors(','Multiple errors are grouped for later error reporting.'),
 ('bearer','request/extractor.go','ExtractToken','BearerExtractor','func (e BearerExtractor) ExtractToken(','HTTP Authorization extraction checks a case-insensitive Bearer prefix.')]:
 a(key,file,symbol,owner,decl);e(key,key,rationale)
a('signed','token.go','SignedString','Token','func (t *Token) SignedString(')
a('signing-input','token.go','SigningString','Token','func (t *Token) SigningString(')
d('signing-call','Which unsigned token serializer does Token.SignedString call?', 'signed','signing-input','sstr, err := t.SigningString()','Token header/claims serialization is distinct from the selected HMAC implementation.')
a('verified','parser.go','ParseWithClaims','Parser','func (p *Parser) ParseWithClaims(')
a('unverified','parser.go','ParseUnverified','Parser','func (p *Parser) ParseUnverified(')
d('verified-call','Which structural token parser does Parser.ParseWithClaims call before verifying the token?', 'verified','unverified','token, parts, err := p.ParseUnverified(tokenString, claims)','Signature and claim verification orchestrates a separate structural parse stage; neither endpoint is an alias or reused control.')
a('date-json','types.go','UnmarshalJSON','NumericDate','func (date *NumericDate) UnmarshalJSON(')
a('date-seconds','types.go','newNumericDateFromSeconds','','func newNumericDateFromSeconds(')
d('date-call','Which fractional-seconds converter does NumericDate.UnmarshalJSON call?', 'date-json','date-seconds','n := newNumericDateFromSeconds(f)','JSON number parsing delegates to fractional Unix-seconds conversion, separate from audience-shape decoding.')

repo('validator')
for key,file,symbol,owner,decl,rationale in [
 ('struct','validator_instance.go','StructCtx','Validate','func (v *Validate) StructCtx(','Struct validation obtains reflected state and traverses declared validation tags.'),
 ('unique','baked_in.go','isUnique','','func isUnique(','Collection uniqueness validates duplicate values or a selected element field using a seen-value map.'),
 ('map','validator_instance.go','ValidateMapCtx','Validate','func (v Validate) ValidateMapCtx(','Map rule validation recursively validates keys against supplied rules.'),
 ('custom-type','validator_instance.go','RegisterCustomTypeFunc','Validate','func (v *Validate) RegisterCustomTypeFunc(','Custom type extraction functions are registered for reflected types.'),
 ('tag-alias','validator_instance.go','RegisterAlias','Validate','func (v *Validate) RegisterAlias(','Tag aliases are validated before updating the alias registry.'),
 ('translation','validator_instance.go','RegisterTranslation','Validate','func (v *Validate) RegisterTranslation(','Translation registration coordinates application registration and translation callbacks.'),
 ('tag-cache','cache.go','Set','tagCache','func (tc *tagCache) Set(','Tag-cache updates clone and atomically publish the lookup map.'),
 ('invalid-error','errors.go','Error','InvalidValidationError','func (e *InvalidValidationError) Error(','Invalid root validation values produce type-aware error messages.'),
 ('credit-card','baked_in.go','isCreditCard','','func isCreditCard(','Credit-card validation applies length/digit and Luhn checksum rules.')]:
 a(key,file,symbol,owner,decl);e(key,key,rationale)
a('var','validator_instance.go','Var','Validate','func (v *Validate) Var(')
a('varctx','validator_instance.go','VarCtx','Validate','func (v *Validate) VarCtx(')
d('variable-call','Which context-aware validator does Validate.Var call?', 'var','varctx','return v.VarCtx(context.Background(), field, tag)','Scalar validation supplies the default context through a concrete receiver.')
a('extract','struct_level.go','ExtractType','validate','func (v *validate) ExtractType(')
a('internal-type','util.go','extractTypeInternal','validate','func (v *validate) extractTypeInternal(')
d('type-call','Which reflection helper is called by validate.ExtractType?', 'extract','internal-type','return v.extractTypeInternal(field, false)','Nullable type unwrapping is separate from registration of custom type callbacks.')
a('numeric-type','util.go','asIntFromType','','func asIntFromType(')
a('duration-parse','util.go','asIntFromTimeDuration','','func asIntFromTimeDuration(')
d('duration-call','Which duration parser is called by asIntFromType for duration fields?', 'numeric-type','duration-parse','return asIntFromTimeDuration(param)','Duration-valued tag parameters take an explicit type-dependent conversion branch.')

repo('mio')
for key,file,symbol,owner,decl,rationale in [
 ('poll','src/poll.rs','poll','Poll','    pub fn poll(','Polling passes the mutable event buffer and timeout to the platform selector.'),
 ('registry-clone','src/poll.rs','try_clone','Registry','    pub fn try_clone(','Registry cloning preserves selector state and the shared debug waker guard.'),
 ('clear-events','src/event/events.rs','clear','Events','    pub fn clear(','Clearing the event buffer is separate from socket registration and polling.'),
 ('listen','src/net/tcp/listener.rs','bind','TcpListener','    pub fn bind(','TCP listener creation sets platform-specific reuse, bind and listen policy.'),
 ('connect','src/net/tcp/stream.rs','connect','TcpStream','    pub fn connect(','A TCP stream starts a nonblocking connection using a newly allocated socket.'),
 ('datagram','src/net/udp.rs','recv_from','UdpSocket','    pub fn recv_from(','Datagram reception returns both payload size and sender address.'),
 ('unwrap','src/io_source.rs','into_inner','IoSource','    pub fn into_inner(','Consuming the I/O wrapper transfers ownership of the underlying resource.'),
 ('associate','src/io_source.rs','associate','SelectorId','    fn associate(','Debug registration association uses an atomic swap and rejects a second association.'),
 ('interests','src/interest.rs','remove','Interest','    pub fn remove(','Removing readiness bits returns None when no nonzero interests remain.')]:
 a(key,file,symbol,owner,decl);e(key,key,rationale)
a('interest-debug','src/interest.rs','fmt','Interest','    fn fmt(&self, fmt:')
a('readable','src/interest.rs','is_readable','Interest','    pub const fn is_readable(')
d('debug-call','Which readable-interest predicate does Interest debug formatting call?', 'interest-debug','readable','if self.is_readable() {','Formatting inspects readiness bits; this pair does not reuse the bit-removal declaration.')
a('peek','src/net/tcp/stream.rs','peek','TcpStream','    pub fn peek(')
a('io','src/io_source.rs','do_io','IoSource','    pub fn do_io<')
d('peek-call','Which IoSource operation wrapper does TcpStream.peek invoke?', 'peek','io','self.inner.do_io(|inner| inner.peek(buf))','The concrete inner IoSource wraps a peeking operation to preserve readiness handling; the closure itself calls the standard stream.')
a('waker','src/waker.rs','new','Waker','    pub fn new(registry:')
a('guard','src/poll.rs','register_waker','Registry','    pub(crate) fn register_waker(')
d('waker-call','Which Registry guard does Waker.new call when debug assertions are enabled?', 'waker','guard','registry.register_waker();','The source relation is explicitly conditional on debug assertions, not an unconditional release execution claim.')

repo('env_logger')
for key,file,symbol,owner,decl,rationale in [
 ('install','src/logger.rs','try_init','Builder','    pub fn try_init(','Global logger installation updates max-level policy only after successful registration.'),
 ('environment','src/logger.rs','parse_env','Builder',"    pub fn parse_env<'a, E>",'Environment configuration merges available filter and style settings.'),
 ('emit','src/logger.rs','log','Logger','    fn log(&self, record:','Record emission manages thread-local/reentrant buffers and formatting failures.'),
 ('format-build','src/fmt/mod.rs','build','Builder','    pub(crate) fn build(','Formatter construction consumes custom/default format configuration once.'),
 ('level-color','src/fmt/mod.rs','default_level_style','Formatter','    pub fn default_level_style(','Log levels receive their configured default terminal styles.'),
 ('indent','src/fmt/mod.rs','write_args','ConfigurableFormatWriter','    fn write_args(','Message formatting applies optional indentation to multiline arguments.'),
 ('writer-build','src/writer/mod.rs','build','Builder','    pub(crate) fn build(','Writer construction resolves auto-color choice and consumes the configured output target.'),
 ('print','src/writer/buffer.rs','print','BufferWriter','    pub(crate) fn print(','Buffered output selects locked streams, test capture or a caller-provided pipe.'),
 ('buffer','src/writer/mod.rs','buffer','Writer','    pub(crate) fn buffer(','A writer allocates an output buffer separately from printing or format selection.')]:
 a(key,file,symbol,owner,decl);e(key,key,rationale)
a('format-entry','src/fmt/mod.rs','format','ConfigurableFormat','    pub fn format(&self, formatter:')
a('format-write','src/fmt/mod.rs','write','ConfigurableFormatWriter','    fn write(mut self, record:')
d('format-call','Which ConfigurableFormatWriter method does ConfigurableFormat.format call?', 'format-entry','format-write','fmt.write(record)','Default record assembly constructs a concrete writer then invokes its ordered header/body writer.')
a('style-entry','src/writer/mod.rs','parse_write_style','Builder','    pub(crate) fn parse_write_style(')
a('style-parser','src/writer/mod.rs','parse_write_style','','fn parse_write_style(',occurrence=1)
d('style-call','Which free style parser is called by writer Builder.parse_write_style?', 'style-entry','style-parser','self.write_style(parse_write_style(write_style))','Configuration text is parsed to an enum before the writer builder is changed.')
a('filter-env','src/logger.rs','get_filter','Env','    fn get_filter(')
a('variable','src/logger.rs','get','Var','    fn get(&self) -> Option<String>')
d('variable-call','Which Var reader is called by Env.get_filter?', 'filter-env','variable','self.filter.get()','The concrete filter field delegates environment-or-default lookup to Var; source inspection executes no environment reads.')

repo('reqwest')
for key,file,symbol,owner,decl,rationale in [
 ('clone','src/async_impl/request.rs','try_clone','Request','    pub fn try_clone(&self) -> Option<Request>','Request cloning rejects uncloneable streaming bodies and copies request metadata.'),
 ('chunk','src/async_impl/response.rs','chunk','Response','    pub async fn chunk(','Streaming consumption skips non-data frames and distinguishes exhaustion from failure.'),
 ('query','src/async_impl/request.rs','query','RequestBuilder','    pub fn query<T:','Query serialization edits URL pairs and preserves serialization failure state.'),
 ('basic-auth','src/util.rs','basic_auth','','pub fn basic_auth<','Basic authorization encodes credentials and marks the resulting header sensitive; no credentials are read or executed.'),
 ('cookie','src/cookie.rs','add_cookie_str','Jar','    pub fn add_cookie_str(','Cookie insertion parses and stores an owned cookie against a URL.'),
 ('redirect-headers','src/redirect.rs','remove_sensitive_headers','','pub(crate) fn remove_sensitive_headers(','Cross-origin redirect transitions remove sensitive authorization and cookie headers.'),
 ('referer','src/redirect.rs','make_referer','','fn make_referer(','Referer construction blocks HTTPS downgrade and strips credentials/fragments.'),
 ('proxy','src/proxy.rs','into_matcher','Proxy','    pub(crate) fn into_matcher(','Proxy configuration is converted to matching policy with auth/header presence caches.'),
 ('timeout','src/error.rs','is_timeout','Error','    pub fn is_timeout(','Timeout classification walks nested error sources and known transport error kinds.')]:
 a(key,file,symbol,owner,decl);e(key,key,rationale)
a('json','src/async_impl/response.rs','json','Response','    pub async fn json<T:')
a('collect','src/async_impl/response.rs','do_bytes','Response','    async fn do_bytes(')
d('json-call','Which byte-collection method does async Response.json call before JSON decoding?', 'json','collect','let (full, url) = self.do_bytes().await?;','JSON decoding first collects the response body and retains its URL for error context.')
a('redirect-check','src/redirect.rs','check','Policy','    pub(crate) fn check(')
a('redirect-policy','src/redirect.rs','redirect','Policy','    pub fn redirect(&self, attempt:')
d('redirect-call','Which redirect policy evaluator does Policy.check call?', 'redirect-check','redirect-policy','self.redirect(Attempt {','The policy adapter constructs an Attempt and invokes concrete custom/limited/none policy evaluation.')
a('headers','src/async_impl/request.rs','headers','RequestBuilder','    pub fn headers(mut self, headers:')
a('merge-headers','src/util.rs','replace_headers','','pub(crate) fn replace_headers(')
d('headers-call','Which header merge helper does RequestBuilder.headers call?', 'headers','merge-headers','crate::util::replace_headers(req.headers_mut(), headers);','Request header-set merging delegates to a package helper while retaining the builder error state.')

repo('petgraph')
for key,file,symbol,owner,decl,rationale in [
 ('node','graph_impl/mod.rs','try_add_node','Graph','    pub fn try_add_node(','Node insertion initializes edge links and checks index capacity.'),
 ('edge','graph_impl/mod.rs','try_add_edge','Graph','    pub fn try_add_edge(','Edge insertion validates endpoints/index bounds and updates adjacency links.'),
 ('remove-node','graph_impl/mod.rs','remove_node','Graph','    pub fn remove_node(','Node removal deletes incident edges and repairs a swap-moved node index.'),
 ('reverse','graph_impl/mod.rs','reverse','Graph','    pub fn reverse(','Graph reversal swaps directed endpoints and incoming/outgoing edge lists.'),
 ('filter','graph_impl/mod.rs','filter_map','Graph',"    pub fn filter_map<'a,",'Subgraph construction maps nodes first and skips edges whose endpoints were removed.'),
 ('toposort','algo/mod.rs','toposort','','pub fn toposort<','Topological sorting uses DFS finish order and rejects directed cycles.'),
 ('astar','algo/astar.rs','astar','','pub fn astar<','A-star uses cost plus heuristic scores and reconstructs the discovered path.'),
 ('union','unionfind.rs','try_union','UnionFind','    pub fn try_union(','Disjoint-set union finds representatives and merges by rank.'),
 ('dot','dot/mod.rs','graph_fmt','Dot','    pub fn graph_fmt<','DOT serialization emits graph/node/edge attributes under explicit configuration.')]:
 a(key,'crates/petgraph/src/'+file,symbol,owner,decl);e(key,key,rationale)
a('dijkstra','crates/petgraph/src/algo/dijkstra.rs','dijkstra','','pub fn dijkstra<')
a('dynamic-goal','crates/petgraph/src/algo/dijkstra.rs','with_dynamic_goal','','pub fn with_dynamic_goal<')
d('shortest-call','Which dynamic-goal engine does dijkstra call?', 'dijkstra','dynamic-goal','with_dynamic_goal(graph, start, |node| goal.as_ref() == Some(node), edge_cost).scores','The fixed optional-goal API delegates to its dynamic-goal search engine; neither endpoint is used by A-star control.')
a('matching','crates/petgraph/src/algo/matching.rs','greedy_matching','','pub fn greedy_matching<')
a('matching-inner','crates/petgraph/src/algo/matching.rs','greedy_matching_inner','','fn greedy_matching_inner<')
d('matching-call','Which matching engine does greedy_matching call before constructing its result?', 'matching','matching-inner','let (mates, n_edges) = greedy_matching_inner(&graph);','Greedy matching computes the mate array before packaging its result; separate from traversal/path algorithms.')
a('path','crates/petgraph/src/algo/mod.rs','has_path_connecting','','pub fn has_path_connecting<')
a('workspace','crates/petgraph/src/algo/mod.rs','with_dfs','','fn with_dfs<')
d('path-call','Which DFS workspace helper is called by has_path_connecting?', 'path','workspace','with_dfs(g, space, |dfs| {','The reachability API directly invokes a workspace helper; nested closure traversal calls are not attributed as this direct relation.')

repo('tanstack-query')
for key,file,symbol,owner,decl,rationale in [
 ('fetch','queryClient.ts','fetchQuery','QueryClient','  fetchQuery<','Fetch orchestration defaults options and chooses cached data versus a stale fetch.'),
 ('optimistic','queryObserver.ts','getOptimisticResult','QueryObserver','  getOptimisticResult(','Optimistic observer results synchronize the current result cursor.'),
 ('mutation','mutation.ts','execute','Mutation','  async execute(','Mutation execution coordinates pending/error/success callbacks and retry state.'),
 ('notifications','notifyManager.ts','createNotifyManager','','export function createNotifyManager(','Notification manager construction creates transactional scheduling and batching state.'),
 ('online','onlineManager.ts','setOnline','OnlineManager','  setOnline(online:','Network status changes notify subscribers only when the boolean state changes.'),
 ('serial-scope','mutationCache.ts','canRun','MutationCache','  canRun(mutation:','Mutation admission allows only the first pending mutation in a named scope.'),
 ('retry','retryer.ts','createRetryer','','export function createRetryer<','Retry controller construction owns cancellation, pausing and failure-count state.'),
 ('key-hash','utils.ts','hashKey','','export function hashKey(','Stable key hashing sorts plain-object property names while encoding JSON.'),
 ('dehydrate','hydration.ts','dehydrate','','export function dehydrate(','Cache dehydration applies query/mutation selection and serialization policy.')]:
 a(key,'packages/query-core/src/'+file,symbol,owner,decl);e(key,key,rationale)
a('destroy','packages/query-core/src/query.ts','destroy','Query','  override destroy():')
a('base-destroy','packages/query-core/src/removable.ts','destroy','Removable','  destroy():')
d('destroy-call','Which Removable base method is explicitly called by Query.destroy?', 'destroy','base-destroy','super.destroy()','The inherited lifecycle call clears the GC timeout before Query cancels its fetch; explicit super binding is source reviewed.')
a('replace','packages/query-core/src/utils.ts','replaceEqualDeep','','export function replaceEqualDeep(a:')
a('array','packages/query-core/src/utils.ts','isPlainArray','','export function isPlainArray(')
d('array-call','Which plain-array predicate does replaceEqualDeep call before structural sharing?', 'replace','array','const array = isPlainArray(a) && isPlainArray(b)','The implemented overload directly tests both inputs before choosing array/object copying; question asks connectivity, not call count.')
a('focus','packages/query-core/src/focusManager.ts','onFocus','FocusManager','  onFocus():')
a('focused','packages/query-core/src/focusManager.ts','isFocused','FocusManager','  isFocused():')
d('focus-call','Which focus-state resolver does FocusManager.onFocus call?', 'focus','focused','const isFocused = this.isFocused()','Focus notification reads the manual-or-document visibility state through a concrete receiver, separate from online state.')

repo('pinia')
for key,file,symbol,decl,occurrence,rationale in [
 ('instance','createPinia.ts','createPinia','export function createPinia(',0,'Instance construction creates effect-scoped state and pending plugin integration.'),
 ('store','store.ts','defineStore','export function defineStore(',0,'Store-definition construction chooses setup/options behavior and builds its useStore closure.'),
 ('hmr','hmr.ts','acceptHMRUpdate','export function acceptHMRUpdate<',0,'Hot-module updates preserve the Pinia instance and reject changed store identifiers.'),
 ('refs','storeToRefs.ts','storeToRefs','export function storeToRefs<',0,'Store-to-ref conversion distinguishes computed values and reactive state.'),
 ('patch','store.ts','mergeReactiveObjects','function mergeReactiveObjects<',0,'Reactive patch merging handles Maps, Sets, nested plain objects and shallow refs.'),
 ('hydrate','store.ts','shouldHydrate','export function shouldHydrate(',0,'Hydration policy respects the explicit skip-hydration marker.'),
 ('actions','mapHelpers.ts','mapActions','export function mapActions<',2,'Options-API action mapping accepts name arrays or renamed mappings and forwards arguments.'),
 ('subscription','subscriptions.ts','addSubscription','export function addSubscription<',0,'Subscriptions install scope disposal and remove callbacks idempotently.'),
 ('devtools','devtools/plugin.ts','registerPiniaDevtools','export function registerPiniaDevtools(',0,'Devtools registration defines inspector, timeline and application integration hooks.')]:
 a(key,'packages/pinia/src/'+file,symbol,'',decl,occurrence=occurrence);e(key,key,rationale)
a('options','packages/pinia/src/store.ts','createOptionsStore','','function createOptionsStore<')
a('setup','packages/pinia/src/store.ts','createSetupStore','','function createSetupStore<')
d('setup-call','Which setup-store constructor is called directly by createOptionsStore?', 'options','setup','store = createSetupStore(id, setup, options, pinia, hot, true)','The options adapter creates state/getter setup and explicitly invokes the shared store constructor; nested setup/getter callback calls are excluded.')
a('plugin','packages/pinia/src/devtools/plugin.ts','devtoolsPlugin','','export function devtoolsPlugin<')
a('add-devtools','packages/pinia/src/devtools/plugin.ts','addStoreToDevtools','','function addStoreToDevtools(')
d('devtools-call','Which store registration helper does devtoolsPlugin call after patching actions?', 'plugin','add-devtools','addStoreToDevtools(','The outer plugin body calls a package-local helper after action grouping; the separate nested HMR callback is not the witness.')
a('tree','packages/pinia/src/devtools/formatting.ts','formatStoreForInspectorTree','','export function formatStoreForInspectorTree(')
a('is-pinia','packages/pinia/src/devtools/utils.ts','isPinia','','export function isPinia(')
d('inspector-call','Which imported Pinia-instance predicate does formatStoreForInspectorTree call?', 'tree','is-pinia','return isPinia(store)','Inspector tree labeling distinguishes the root Pinia instance from a store with an explicitly imported predicate.')

repo('reselect')
for key,file,symbol,decl,occurrence,rationale in [
 ('selector','createSelectorCreator.ts','createSelectorCreator','export function createSelectorCreator<',2,'Selector factory configuration binds memoizers and per-selector options.'),
 ('structured','createStructuredSelector.ts','createStructuredSelector','export const createStructuredSelector:',0,'Structured selectors compose named selector results into a matching object.'),
 ('collect','utils.ts','collectInputSelectorResults','export function collectInputSelectorResults(',0,'Input selector evaluation collects each dependency result using the supplied arguments.'),
 ('development-config','devModeChecks/setGlobalDevModeChecks.ts','setGlobalDevModeChecks','export const setGlobalDevModeChecks =',0,'Global development-check policy merges partial frequency settings.'),
 ('identity-warning','devModeChecks/identityFunctionCheck.ts','runIdentityFunctionCheck','export const runIdentityFunctionCheck =',0,'Identity-function diagnostics probe an output function that returned its input unchanged.'),
 ('cache-warning','devModeChecks/cacheSizeCheck.ts','runCacheSizeCheck','export const runCacheSizeCheck =',0,'Unbounded primitive-key cache diagnostics report size and a captured stack.'),
 ('array','utils.ts','ensureIsArray','export const ensureIsArray =',0,'Argument normalization retains an array or wraps a scalar as one element.'),
 ('weak-ref','weakMapMemoize.ts','getWeakRef','const getWeakRef =',0,'Weak-reference support detection selects native WeakRef or a strong-reference compatibility class.'),
 ('lru-cache','lruMemoize.ts','createLruCache','function createLruCache(',0,'An LRU cache owns recency promotion, bounded insertion and result-equivalence lookup.')]:
 a(key,'src/'+file,symbol,'',decl,occurrence=occurrence);e(key,key,rationale)
a('lru','src/lruMemoize.ts','lruMemoize','','export function lruMemoize<')
a('comparator','src/lruMemoize.ts','createCacheKeyComparator','','export function createCacheKeyComparator(')
d('comparator-call','Which argument comparator factory does lruMemoize call during construction?', 'lru','comparator','const comparator = createCacheKeyComparator(equalityCheck)','The outer memoizer initializes an argument comparator; calls made later inside the memoized closure are excluded.')
a('weak','src/weakMapMemoize.ts','weakMapMemoize','','export function weakMapMemoize<')
a('cache-node','src/weakMapMemoize.ts','createCacheNode','','function createCacheNode<')
d('weak-call','Which cache-node constructor is called directly when weakMapMemoize is initialized?', 'weak','cache-node','let fnNode = createCacheNode()','Initial WeakMap-tree state is created in the factory body; later generation-flip callback calls are not used as the witness.')
TASKS[-1]['selectedDirectWitnessOnly'] = True
a('dependencies','src/utils.ts','getDependencies','','export function getDependencies(')
a('assert-dependencies','src/utils.ts','assertIsArrayOfFunctions','','export function assertIsArrayOfFunctions<')
d('dependencies-call','Which array-of-functions validator does getDependencies call?', 'dependencies','assert-dependencies','assertIsArrayOfFunctions<Selector>(','Selector dependency extraction validates the normalized input list through a concrete package helper.')

repo('immer')
for key,file,symbol,owner,decl,rationale in [
 ('produce','core/immerClass.ts','produce','Immer','\tproduce: IProduce =','Immutable update orchestration distinguishes curried, draftable and primitive inputs.'),
 ('auto-freeze','core/immerClass.ts','setAutoFreeze','Immer','\tsetAutoFreeze(','The Immer instance stores explicit automatic freezing policy.'),
 ('freeze','utils/common.ts','freeze','','export function freeze<T>(obj: any,','Freezing guards drafts/frozen values and disables Map/Set mutation before optional deep recursion.'),
 ('plain','utils/common.ts','isPlainObject','','export function isPlainObject(','Plain-object detection compares prototypes/constructors with cached constructor strings.'),
 ('plugin','utils/plugins.ts','getPlugin','','export function getPlugin<','Plugin lookup rejects missing optional implementation registrations.'),
 ('patches','plugins/patches.ts','enablePatches','','export function enablePatches(','Patch-plugin activation registers patch generation/application behavior and development errors.'),
 ('mapset','plugins/mapset.ts','enableMapSet','','export function enableMapSet(','Map/Set plugin activation creates specialized draft collection implementations.'),
 ('finalize','core/finalize.ts','processResult','','export function processResult(','Recipe results are finalized or replaced, frozen, revoked and optionally reported as patches.'),
 ('copy','utils/common.ts','shallowCopy','','export function shallowCopy(','Copying distinguishes collection/array/plain/class objects and strict descriptor semantics.')]:
 a(key,'src/'+file,symbol,owner,decl);e(key,key,rationale)
a('current','src/core/current.ts','current','','export function current(value:')
a('current-impl','src/core/current.ts','currentImpl','','function currentImpl(')
d('snapshot-call','Which implementation helper does current call after validating a draft?', 'current','current-impl','return currentImpl(value)','Snapshot creation enters a recursive implementation after validating the input draft.')
a('revoke','src/core/scope.ts','revokeScope','','export function revokeScope(')
a('leave','src/core/scope.ts','leaveScope','','export function leaveScope(')
d('scope-call','Which scope restoration helper does revokeScope call before revoking drafts?', 'revoke','leave','leaveScope(scope)','Scope revocation restores the parent current scope before invalidating drafts.')
a('proxy','src/core/immerClass.ts','createProxy','','export function createProxy<')
a('object-proxy','src/core/proxy.ts','createProxyProxy','','export function createProxyProxy<')
d('proxy-call','Which object/array proxy constructor does createProxy call for values that are neither Maps nor Sets?', 'proxy','object-proxy',': createProxyProxy(value, parent)','The object/array branch directly invokes a statically imported constructor; Map/Set plugin dispatch is a different branch.')
