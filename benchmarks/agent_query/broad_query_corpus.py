"""Source-authored Plan 029 panel. This module never reads tool output.

Selectors below are review aids, not a language parser or proof of semantics.
The freezer checks the pinned bytes; a second reviewer must assess facts/absence.
"""
REPOSITORIES = {
    'itsdangerous': ('python', 'pallets/itsdangerous'),
    'marshmallow': ('python', 'marshmallow-code/marshmallow'),
    'httprouter': ('go', 'julienschmidt/httprouter'),
    'go-cache': ('go', 'patrickmn/go-cache'),
    'log': ('rust', 'rust-lang/log'),
    'humantime': ('rust', 'tailhook/humantime'),
    'p-queue': ('typescript', 'sindresorhus/p-queue'),
    'superjson': ('typescript', 'flightcontrolhq/superjson'),
}
ANCHORS = {}
TASKS = []
REPO = None


def repo(name):
    global REPO
    REPO = name


def a(key, file, symbol, owner, declaration, *witnesses, occurrence=0):
    ANCHORS[(REPO, key)] = dict(file=file, symbol=symbol, owner=owner,
        declaration=declaration, occurrence=occurrence,
        witnesses=[dict(needle=n, statement=s) for n, s in witnesses])


def task(kind, key, question, anchors=(), *, cluster=None, edges=(), rationale=None):
    TASKS.append(dict(id=f'{REPO}.{key}', repository=REPO,
        cluster=f'{REPO}.{cluster or key}', kind=kind, question=question,
        anchors=list(anchors), edges=list(edges), rationale=rationale))


def b(key, question, *anchors, cluster=None):
    task('broad', key, question, anchors, cluster=cluster)


def e(key, question, anchor, cluster=None):
    task('exact', key, question, [anchor], cluster=cluster)


def d(key, question, source, target, call, cluster=None):
    task('directed', key, question, [source, target], edges=[(source, target, call)], cluster=cluster)


def m(key, question, nodes, calls, cluster=None):
    task('multihop', key, question, nodes,
         edges=[(nodes[i], nodes[i+1], call) for i, call in enumerate(calls)], cluster=cluster)


def u(key, question, *anchors):
    task('ambiguous', key, question, anchors)


def n(key, question, rationale):
    task('unsupported', key, question, rationale=rationale)


repo('itsdangerous')
a('bytes', 'src/itsdangerous/encoding.py', 'want_bytes', '', 'def want_bytes(',
  ('s = s.encode(encoding, errors)', 'Text input is encoded using the requested encoding and error policy.'))
a('b64e', 'src/itsdangerous/encoding.py', 'base64_encode', '', 'def base64_encode(',
  ('return base64.urlsafe_b64encode(string).rstrip(b"=")', 'URL-safe encoding removes trailing padding.'))
a('b64d', 'src/itsdangerous/encoding.py', 'base64_decode', '', 'def base64_decode(',
  ('string += b"=" * (-len(string) % 4)', 'Decoding restores padding to a multiple of four.'))
a('verify', 'src/itsdangerous/signer.py', 'verify_signature', 'Signer', '    def verify_signature(self, value:',
  ('for secret_key in reversed(self.secret_keys):', 'Verification tries configured keys newest first.'))
a('derive', 'src/itsdangerous/signer.py', 'derive_key', 'Signer', '    def derive_key(',
  ('self.salt + b"signer" + secret_key', 'Django-style derivation includes salt and the signer marker.'))
a('compress', 'src/itsdangerous/url_safe.py', 'dump_payload', 'URLSafeSerializerMixin', '    def dump_payload(',
  ('if len(compressed) < (len(json) - 1):', 'Compression is selected only when it saves more than the marker byte.'),
  ('base64d = b"." + base64d', 'A leading dot marks compressed payloads.'))
a('decompress', 'src/itsdangerous/url_safe.py', 'load_payload', 'URLSafeSerializerMixin', '    def load_payload(',
  ('if payload.startswith(b"."):', 'The leading dot selects decompression.'),
  ('json = zlib.decompress(json)', 'Decoded marked data is decompressed before deserialization.'))
a('timeunsign', 'src/itsdangerous/timed.py', 'unsign', 'TimestampSigner', '    def unsign(',
  ('if age > max_age:', 'Timestamps older than max_age are rejected.'),
  ('if age < 0:', 'Timestamps from the future are rejected when checking age.'), occurrence=2)
a('ct', 'src/itsdangerous/signer.py', 'verify_signature', 'SigningAlgorithm', '    def verify_signature(self, key:',
  ('return hmac.compare_digest(sig, self.get_signature(key, value))', 'Signature comparison uses compare_digest.'))
a('fallback', 'src/itsdangerous/serializer.py', 'iter_unsigners', 'Serializer', '    def iter_unsigners(',
  ('for fallback in self.fallback_signers:', 'Deserialization can try configured fallback signer configurations.'))
a('unsafe', 'src/itsdangerous/serializer.py', '_loads_unsafe_impl', 'Serializer', '    def _loads_unsafe_impl(',
  ('self.load_payload(e.payload, **load_payload_kwargs)', 'Unsafe loading may expose a payload whose signature did not validate.'))
a('sig', 'src/itsdangerous/signer.py', 'get_signature', 'Signer', '    def get_signature(self, value:',
  ('key = self.derive_key()', 'Signing derives a key before producing the signature.'))
a('clock', 'src/itsdangerous/timed.py', 'get_timestamp', 'TimestampSigner', '    def get_timestamp(',
  ('return int(time.time())', 'The signing timestamp is integer epoch seconds.'))
a('loads', 'src/itsdangerous/serializer.py', 'loads', 'Serializer', '    def loads(',
  ('for signer in self.iter_unsigners(salt):', 'Loading tries the serializer unsigners.'), occurrence=1)
a('timeloads', 'src/itsdangerous/timed.py', 'loads', 'TimedSerializer', '    def loads(',
  ('for signer in self.iter_unsigners(salt):', 'Timed loading also iterates unsigners.'))
a('unsign', 'src/itsdangerous/signer.py', 'unsign', 'Signer', '    def unsign(',
  ('if self.verify_signature(value, sig):', 'Unsigning verifies the separated signature before returning the payload.'))
b('text', 'Where is text converted to bytes before signing or encoding?', 'bytes')
b('padding', 'Where is missing URL-safe Base64 padding restored?', 'b64d')
b('rotation', 'Where does signature verification try older keys during key rotation?', 'verify')
b('salt', 'Where does salted key derivation combine the signer marker with the secret?', 'derive')
b('compression', 'Where is payload compression selected and marked?', 'compress')
b('decompression', 'Where are compressed URL-safe payloads recognized and unpacked?', 'decompress')
b('expiration', 'Where are expired or future-dated signed timestamps rejected?', 'timeunsign')
b('constant-time', 'Where are signature bytes compared with a timing-resistant primitive?', 'ct')
b('fallback', 'Where are fallback signing algorithms tried while reading old data?', 'fallback')
b('unsafe-load', 'Where can invalidly signed data still be decoded for inspection?', 'unsafe')
e('exact-signature', 'Find Signer.get_signature in signer.py.', 'sig')
e('exact-clock', 'Find TimestampSigner.get_timestamp.', 'clock')
d('encoding-call', 'Which encoder does URLSafeSerializerMixin.dump_payload call?', 'compress', 'b64e', 'base64d = base64_encode(json)', cluster='compression')
d('derive-call', 'Which key derivation method is called by Signer.get_signature?', 'sig', 'derive', 'key = self.derive_key()', cluster='salt')
m('encoding-chain', 'Trace URLSafeSerializerMixin.dump_payload through Base64 encoding to byte conversion.', ['compress','b64e','bytes'], ['base64d = base64_encode(json)', 'string = want_bytes(string)'], cluster='compression')
m('signature-chain', 'Trace Signer.get_signature through Base64 signature encoding to byte coercion.', ['sig','b64e','bytes'], ['return base64_encode(sig)', 'string = want_bytes(string)'], cluster='salt')
u('ambiguous-loads', 'Find loads without specifying a serializer type.', 'loads', 'timeloads')
u('ambiguous-unsign', 'Find unsign without specifying a signer type.', 'unsign', 'timeunsign')
n('revocation', 'Where is the server-side token revocation database updated?', 'The pinned package signs and verifies self-contained payloads. No server-side revocation database implementation exists in its tracked source scope; application storage is outside this package.')
n('rsa', 'Where are RSA public/private key pairs generated?', 'The tracked signing implementations use configured symmetric secrets and digest algorithms; asymmetric RSA key generation is not implemented here.')

repo('marshmallow')
a('dump', 'src/marshmallow/schema.py', 'dump', 'Schema', '    def dump(',
  ('if self._hooks[PRE_DUMP]:', 'Pre-dump processors run before serialization.'),
  ('if self._hooks[POST_DUMP]:', 'Post-dump processors run after serialization.'))
a('serialize', 'src/marshmallow/schema.py', '_serialize', 'Schema', '    def _serialize(',
  ('if value is missing:', 'Missing serialized values are omitted.'),
  ('key = field_obj.data_key if field_obj.data_key is not None else attr_name', 'Output field names honor data_key.'))
a('deserialize', 'src/marshmallow/schema.py', '_deserialize', 'Schema', '    def _deserialize(',
  ('if partial is True or (', 'Partial loads skip permitted missing fields.'),
  ('if unknown == INCLUDE:', 'Unknown included fields are copied.'),
  ('elif unknown == RAISE:', 'Unknown rejected fields are stored as errors.'))
a('missing', 'src/marshmallow/fields.py', '_validate_missing', 'Field', '    def _validate_missing(',
  ('if value is missing_ and self.required:', 'Required missing values raise validation errors.'),
  ('if value is None and not self.allow_none:', 'Null values are rejected unless allowed.'))
a('fieldload', 'src/marshmallow/fields.py', 'deserialize', 'Field', '    def deserialize(',
  ('return _miss() if callable(_miss) else _miss', 'Missing values use the configured load default.'),
  ('for func in self.pre_load:', 'Pre-load field hooks transform input.'),
  ('for func in self.post_load:', 'Post-load field hooks transform deserialized output.'), occurrence=2)
a('merge', 'src/marshmallow/error_store.py', 'merge_errors', '', 'def merge_errors(',
  ('errors1[key] = merge_errors(errors1[key], val)', 'Nested error dictionaries are merged recursively.'))
a('class', 'src/marshmallow/class_registry.py', 'get_class', '', 'def get_class(',
  ('if len(classes) > 1:', 'Unqualified names with multiple registered classes are ambiguous.'), occurrence=2)
a('value', 'src/marshmallow/utils.py', 'get_value', '', 'def get_value(',
  ('for k in key.split("."):', 'Dotted lookup traverses each key component.'))
a('set', 'src/marshmallow/utils.py', 'set_value', '', 'def set_value(',
  ('target = dct.setdefault(head, {})', 'Dotted assignment creates intermediate dictionaries.'))
a('dumps', 'src/marshmallow/schema.py', 'dumps', 'Schema', '    def dumps(',
  ('serialized = self.dump(obj, many=many)', 'JSON output first dumps through the schema.'))
a('loads', 'src/marshmallow/schema.py', 'loads', 'Schema', '    def loads(',
  ('data = self.opts.render_module.loads(s, **kwargs)', 'JSON input is decoded before schema loading.'))
a('load', 'src/marshmallow/schema.py', 'load', 'Schema', '    def load(',
  ('return self._do_load(', 'Schema loading delegates to the load orchestration.'))
a('doload', 'src/marshmallow/schema.py', '_do_load', 'Schema', '    def _do_load(',
  ('error_store = ErrorStore()', 'Load orchestration starts an error store.'))
a('basefieldserialize', 'src/marshmallow/fields.py', '_serialize', 'Field', '    def _serialize(',
  ('return value', 'Base field serialization returns the supplied value.'))
a('nestedserialize', 'src/marshmallow/fields.py', '_serialize', 'Nested', '    def _serialize(self, nested_obj, attr, obj, **kwargs):',
  ('return schema.dump(nested_obj, many=many)', 'Nested serialization dumps via its nested schema.'))
a('basefielddeserialize','src/marshmallow/fields.py','_deserialize','Field','    def _deserialize(',('return value','The base field hook returns its input.'))
a('nesteddeserialize','src/marshmallow/fields.py','_deserialize','Nested','    def _deserialize(',('self._test_collection(value)','Nested loading checks the expected collection shape.'),occurrence=1)
a('collection', 'src/marshmallow/utils.py', 'is_collection', '', 'def is_collection(',
  ('not isinstance(obj, Mapping)', 'Mappings are excluded from collections.'))
b('dump-hooks', 'Where do schema hooks run before and after serialization?', 'dump')
b('output-fields', 'Where are missing output values skipped and external field names chosen?', 'serialize')
b('partial', 'Where does partial loading allow missing fields?', 'deserialize')
b('unknown', 'Where are unknown input fields included or reported as errors?', 'deserialize', cluster='partial')
b('required', 'Where are required values and disallowed nulls checked?', 'missing')
b('default', 'Where are missing field values replaced by load defaults?', 'fieldload')
b('field-hooks', 'Where are per-field pre-load and post-load transformations applied?', 'fieldload', cluster='default')
b('nested-errors', 'Where are nested validation error dictionaries combined?', 'merge')
b('dotted-read', 'Where does a dotted field name traverse nested object values?', 'value')
b('dotted-write', 'Where does dotted assignment create intermediate dictionaries?', 'set')
e('exact-class', 'Find get_class in class_registry.py.', 'class')
e('exact-collection', 'Find is_collection in utils.py.', 'collection')
d('dumps-call', 'Which schema method does Schema.dumps call before rendering JSON?', 'dumps','dump','serialized = self.dump(obj, many=many)', cluster='dump-hooks')
d('load-call', 'Which orchestration method does Schema.load delegate to?', 'load','doload','return self._do_load(')
m('dump-chain', 'Trace Schema.dumps through dump to _serialize.', ['dumps','dump','serialize'], ['serialized = self.dump(obj, many=many)','result = self._serialize(processed_obj, many=many)'], cluster='dump-hooks')
m('load-chain', 'Trace Schema.loads through load to _do_load.', ['loads','load','doload'], ['return self.load(', 'return self._do_load('], cluster='load-call')
u('ambiguous-serialize', 'Find _serialize without specifying a field or schema type.', 'basefieldserialize','nestedserialize','serialize')
u('ambiguous-load', 'Find _deserialize without specifying the declaring type.', 'basefielddeserialize','nesteddeserialize','deserialize')
n('migration', 'Where does the built-in SQL database migration executor apply schema migrations?', 'This is a serialization/validation library. Its tracked source has no SQL database migration executor; a marshmallow Schema is not a database schema migration.')
n('remote-registry', 'Where does the remote JSON Schema registry client download schemas?', 'The package resolves local registered Python schema classes; its tracked source does not implement a remote JSON Schema registry client.')

repo('httprouter')
a('byname','router.go','ByName','Params','func (ps Params) ByName(',('if p.Key == name {','Parameters are searched by key name.'))
a('context','router.go','ParamsFromContext','','func ParamsFromContext(',('p, _ := ctx.Value(ParamsKey).(Params)','Route parameters are retrieved from the request context key.'))
a('handle','router.go','Handle','Router','func (r *Router) Handle(',('if method == "" {','Empty methods are rejected.'),('if len(path) < 1 || path[0] != \'/\' {','Paths must begin with a slash.'))
a('matched','router.go','saveMatchedRoutePath','Router','func (r *Router) saveMatchedRoutePath(',('Key: MatchedRoutePathParam','The registered route path is appended as a special parameter.'))
a('serve','router.go','ServeHTTP','Router','func (r *Router) ServeHTTP(',('handle(w, req, *ps)','Matched handlers receive route parameters.'),('code := http.StatusMovedPermanently','GET redirection starts with permanent redirect status.'),('code = http.StatusPermanentRedirect','Non-GET redirect preserves the method using status 308.'))
a('allowed','router.go','allowed','Router','func (r *Router) allowed(',('return strings.Join(allowed, ", ")','Allowed method names form a comma-separated header value.'))
a('recover','router.go','recv','Router','func (r *Router) recv(',('if rcv := recover(); rcv != nil {','A recovered panic is passed to the configured handler.'))
a('clean','path.go','CleanPath','','func CleanPath(',('case p[r] == \'.\' && r+1 == n:','A terminal dot segment is removed.'),('case p[r] == \'.\' && p[r+1] == \'.\'','Parent segments backtrack toward a preceding path segment.'))
a('insert','tree.go','insertChild','node','func (n *node) insertChild(',('panic("only one wildcard per path segment is allowed','Multiple wildcard markers in a segment are rejected.'),('panic("catch-all routes are only allowed at the end','Catch-all routes must occur at the path end.'))
a('wildcard','tree.go','findWildcard','','func findWildcard(',('if c != \':\' && c != \'*\' {','Colon and star introduce parameter or catch-all wildcards.'))
a('count','tree.go','countParams','','func countParams(',('case \':\', \'*\':','Parameter markers are counted.'))
a('lookup','router.go','Lookup','Router','func (r *Router) Lookup(',('handle, ps, tsr := root.getValue(path, r.getParams)','Manual lookup returns the tree match.'))
a('get','router.go','GET','Router','func (r *Router) GET(',('r.Handle(http.MethodGet, path, handle)','GET registration delegates to Handle.'))
a('handlerfunc','router.go','HandlerFunc','Router','func (r *Router) HandlerFunc(',('r.Handler(method, path, handler)','HandlerFunc delegates to Handler.'))
a('handler','router.go','Handler','Router','func (r *Router) Handler(',('r.Handle(method, path,','Standard HTTP handlers are adapted to router handlers.'))
a('addroute','tree.go','addRoute','node','func (n *node) addRoute(',('n.priority++','Route insertion raises node priority.'))
a('ci','tree.go','findCaseInsensitivePath','node','func (n *node) findCaseInsensitivePath(',('ciPath := n.findCaseInsensitivePathRec(','Case-insensitive lookup delegates to recursive search.'))
a('cirec','tree.go','findCaseInsensitivePathRec','node','func (n *node) findCaseInsensitivePathRec(',('rb = shiftNRuneBytes(rb, npLen)','Recursive search shifts consumed rune bytes.'))
a('shift','tree.go','shiftNRuneBytes','','func shiftNRuneBytes(',('switch n {','Rune-byte shifting branches on byte count.'))
a('testserve','router_test.go','ServeHTTP','handlerStruct','func (h handlerStruct) ServeHTTP(',('*h.handled = true','The test HTTP handler records a hit.'))
a('testparams','tree_test.go','getParams','','func getParams(',('return &ps','The test parameter allocator returns a parameter slice pointer.'))
a('params','router.go','getParams','Router','func (r *Router) getParams(',('ps, _ := r.paramsPool.Get().(*Params)','Router parameter slices come from its pool.'))
b('parameter','Where is a route parameter retrieved by its name?', 'byname')
b('context','Where are captured route parameters read from an HTTP request context?', 'context')
b('registration','Where are invalid method and path registrations rejected?', 'handle')
b('matched-route','Where is the original matched route path retained in the parameters?', 'matched')
b('dispatch','Where is a matching request dispatched to its registered handler?', 'serve')
b('redirect','Where are trailing-slash redirects assigned GET and non-GET status codes?', 'serve', cluster='dispatch')
b('allow','Where are allowed HTTP methods assembled for response headers?', 'allowed')
b('panic','Where are request handler panics recovered for a configured error handler?', 'recover')
b('clean-path','Where are dot and parent path segments cleaned?', 'clean')
b('wildcard','Where are invalid wildcard and catch-all route placements rejected?', 'insert')
e('exact-count','Find countParams in tree.go.', 'count')
e('exact-lookup','Find Router.Lookup.', 'lookup')
d('get-call','Which registration method is called by Router.GET?', 'get','handle','r.Handle(http.MethodGet, path, handle)', cluster='registration')
d('handler-call','Which adapter does Router.HandlerFunc call?', 'handlerfunc','handler','r.Handler(method, path, handler)')
m('register-chain','Trace Router.GET through Handle into radix-tree route insertion.', ['get','handle','addroute'], ['r.Handle(http.MethodGet, path, handle)','root.addRoute(path, handle)'], cluster='registration')
m('case-chain','Trace case-insensitive path lookup through recursive lookup to rune-byte shifting.', ['ci','cirec','shift'], ['ciPath := n.findCaseInsensitivePathRec(', 'rb = shiftNRuneBytes(rb, npLen)'])
u('ambiguous-http','Find ServeHTTP without specifying the owner, including test source.', 'serve','testserve')
u('ambiguous-params','Find getParams without specifying a file or owner, including test source.', 'params','testparams')
n('jwt','Where does the built-in JWT authentication middleware verify access tokens?', 'This router matches paths and methods and adapts handlers. Its tracked source does not supply a JWT authentication middleware implementation.')
n('database','Where does the built-in SQL connection pool execute route queries?', 'No SQL connection pool or database execution layer is implemented in the tracked router source; application handlers are supplied by users.')

repo('go-cache')
a('set','cache.go','Set','cache','func (c *cache) Set(',('if d == DefaultExpiration {','The configured default TTL is substituted.'),('e = time.Now().Add(d).UnixNano()','Positive expiration durations become absolute nanosecond deadlines.'))
a('privateset','cache.go','set','cache','func (c *cache) set(',('if d == DefaultExpiration {','The private setter also substitutes the configured default TTL.'),('e = time.Now().Add(d).UnixNano()','The private setter also computes an absolute deadline.'))
a('privateget','cache.go','get','cache','func (c *cache) get(',('if time.Now().UnixNano() > item.Expiration {','The private lookup also hides expired items.'))
a('add','cache.go','Add','cache','func (c *cache) Add(',('if found {','Adding an existing live key fails.'))
a('replace','cache.go','Replace','cache','func (c *cache) Replace(',('if !found {','Replacing an absent key fails.'))
a('get','cache.go','Get','cache','func (c *cache) Get(',('if time.Now().UnixNano() > item.Expiration {','Expired items are hidden during lookup.'))
a('getexp','cache.go','GetWithExpiration','cache','func (c *cache) GetWithExpiration(',('return item.Object, time.Unix(0, item.Expiration), true','Lookup can return the absolute expiration time.'),('if time.Now().UnixNano() > item.Expiration {','Lookup with expiration also hides expired items.'))
a('expired','cache.go','DeleteExpired','cache','func (c *cache) DeleteExpired(',('c.mu.Unlock()','The cache lock is released before eviction callbacks.'),('c.onEvicted(v.key, v.value)','Callbacks run for collected expired items.'))
a('save','cache.go','Save','cache','func (c *cache) Save(',('err = enc.Encode(&c.items)','Persistence encodes the item map with gob.'))
a('load','cache.go','Load','cache','func (c *cache) Load(',('if !found || ov.Expired() {','Loaded values replace only missing or expired existing values.'))
a('items','cache.go','Items','cache','func (c *cache) Items(',('if now > v.Expiration {','Item snapshots omit expired entries.'))
a('count','cache.go','ItemCount','cache','func (c *cache) ItemCount(',('n := len(c.items)','ItemCount counts stored entries including those not yet removed.'))
a('default','cache.go','SetDefault','cache','func (c *cache) SetDefault(',('c.Set(k, x, DefaultExpiration)','SetDefault delegates with the default-expiration sentinel.'))
a('savefile','cache.go','SaveFile','cache','func (c *cache) SaveFile(',('err = c.Save(fp)','File persistence delegates to Save.'))
a('loadfile','cache.go','LoadFile','cache','func (c *cache) LoadFile(',('err = c.Load(fp)','File restore delegates to Load.'))
a('itemexpired','cache.go','Expired','Item','func (item Item) Expired(',('return time.Now().UnixNano() > item.Expiration','Item expiration compares the current time with its deadline.'))
a('new','cache.go','New','','func New(',('return newCacheWithJanitor(defaultExpiration, cleanupInterval, items)','Construction delegates with an empty item map.'))
a('withjanitor','cache.go','newCacheWithJanitor','','func newCacheWithJanitor(',('c := newCache(de, m)','The janitor wrapper creates the underlying cache.'))
a('newcache','cache.go','newCache','','func newCache(',('de = -1','A zero default duration becomes no expiration.'))
a('shardset','sharded.go','Set','shardedCache','func (sc *shardedCache) Set(',('sc.bucket(k).Set(k, x, d)','Sharded Set delegates to the selected bucket.'))
a('shardget','sharded.go','Get','shardedCache','func (sc *shardedCache) Get(',('return sc.bucket(k).Get(k)','Sharded Get delegates to the selected bucket.'))
b('ttl','Which cache setter implementations convert default TTLs into absolute expiration deadlines?', 'set','privateset')
b('add','Where does adding a key fail if a live item already exists?', 'add')
b('replace','Where does replacing a key fail if it is absent?', 'replace')
b('stale-read','Where do the cache lookup implementations hide expired values from reads?', 'get','privateget','getexp')
b('deadline','Where can a cache read return an item and its expiration timestamp?', 'getexp')
b('eviction','Where are expired entries removed and callbacks invoked after unlocking?', 'expired')
b('persist','Where is the in-memory item map encoded for persistence?', 'save')
b('restore','Where does loading preserve an existing unexpired cached item?', 'load')
b('snapshot','Where is a snapshot made containing only live cache entries?', 'items')
b('count','Where are stored items counted even when expired items remain?', 'count')
e('exact-default','Find cache.SetDefault.', 'default', cluster='ttl')
e('exact-savefile','Find cache.SaveFile.', 'savefile', cluster='persist')
d('default-call','Which setter is called by cache.SetDefault?', 'default','set','c.Set(k, x, DefaultExpiration)', cluster='ttl')
d('save-call','Which serializer is called by cache.SaveFile?', 'savefile','save','err = c.Save(fp)', cluster='persist')
m('construct-chain','Trace New through the janitor wrapper to underlying cache construction.', ['new','withjanitor','newcache'], ['return newCacheWithJanitor(defaultExpiration, cleanupInterval, items)','c := newCache(de, m)'])
m('restore-chain','Trace LoadFile through Load to the existing-item expiration check.', ['loadfile','load','itemexpired'], ['err = c.Load(fp)','if !found || ov.Expired() {'], cluster='restore')
u('ambiguous-set','Find Set without specifying whether the cache is sharded.', 'set','shardset')
u('ambiguous-get','Find Get without specifying whether the cache is sharded.', 'get','shardget')
n('redis-replication','Where does the built-in Redis replication client synchronize cache keys?', 'The tracked implementation is an in-process map cache, with optional sharding and gob persistence. It has no Redis replication client.')
n('wal','Where does the transactional write-ahead log commit each cache mutation?', 'Optional gob snapshots exist, but no transactional write-ahead logging implementation is present in the tracked source.')

repo('log')
a('parse','src/lib.rs','from_str','Level','    fn from_str(level: &str) -> Result<Level, Self::Err>',('for idx in 1..LOG_LEVEL_NAMES.len() {','Level parsing excludes Off.'),('if LOG_LEVEL_NAMES[idx].eq_ignore_ascii_case(level) {','Level names are parsed case-insensitively.'))
a('filter','src/lib.rs','to_level','LevelFilter','    pub fn to_level(&self) -> Option<Level>',('Level::from_usize(*self as usize)','The Off filter has no corresponding logging Level.'))
a('iter','src/lib.rs','iter','Level','    pub fn iter() -> impl Iterator<Item = Self>',('(1..6).map(|i| Self::from_usize(i).unwrap())','Iteration covers the five enabled levels from Error through Trace.'))
a('increase','src/lib.rs','increment_severity','Level','    pub fn increment_severity(&self) -> Self',('Self::from_usize(current + 1).unwrap_or(*self)','Increasing at the upper bound leaves the level unchanged.'))
a('filterincrease','src/lib.rs','increment_severity','LevelFilter','    pub fn increment_severity(&self) -> Self',('Self::from_usize(current + 1).unwrap_or(*self)','Increasing a filter at the upper bound leaves it unchanged.'),occurrence=1)
a('setmaxracy','src/lib.rs','set_max_level_racy','','pub unsafe fn set_max_level_racy(',('MAX_LOG_LEVEL_FILTER.store(level as usize, Ordering::Relaxed);','The racy setter updates the same filter and requires the caller to prevent concurrent updates.'))
a('recordlevelset','src/lib.rs','level','RecordBuilder','    pub fn level(&mut self, level:',('self.record.metadata.level = level;','The record builder level setter changes its metadata.'))
a('metalevelset','src/lib.rs','level','MetadataBuilder','    pub fn level(&mut self, arg:',('self.metadata.level = arg;','The metadata builder level setter changes its stored level.'))
a('recordnew','src/lib.rs','new','RecordBuilder','    pub fn new() -> RecordBuilder',('metadata: Metadata::builder().build(),','A new record builder starts with default metadata.'),('module_path: None,','The module path starts unset.'))
a('recordfile','src/lib.rs','file_static','RecordBuilder','    pub fn file_static(&mut self, file:',('self.record.file = file.map(MaybeStaticStr::Static);','Static file paths retain their static classification.'))
a('setmax','src/lib.rs','set_max_level','','pub fn set_max_level(',('MAX_LOG_LEVEL_FILTER.store(level as usize, Ordering::Relaxed);','The global maximum level is updated atomically with relaxed ordering.'))
a('max','src/lib.rs','max_level','','pub fn max_level(',('mem::transmute(MAX_LOG_LEVEL_FILTER.load(Ordering::Relaxed))','The current maximum level is read from the atomic filter.'))
a('installinner','src/lib.rs','set_logger_inner','','fn set_logger_inner<F>',('match STATE.compare_exchange(','Initialization reserves the uninitialized logger state atomically.'),('STATE.store(INITIALIZED, Ordering::Release);','Logger publication uses release ordering.'))
a('logger','src/lib.rs','logger','','pub fn logger()',('if STATE.load(Ordering::Acquire) != INITIALIZED {','Logger access checks publication state with acquire ordering.'),('&NOP','Before initialization access returns a no-op logger.'))
a('recordbuild','src/lib.rs','build','RecordBuilder','    pub fn build(&self) -> Record',('self.record.clone()','Building clones the assembled record.'))
a('metanew','src/lib.rs','new','MetadataBuilder','    pub fn new() -> MetadataBuilder',('level: Level::Info,','Default metadata begins at Info level.'))
a('recordbuilder','src/lib.rs','builder','Record','    pub fn builder() -> RecordBuilder',('RecordBuilder::new()','Record::builder delegates to its builder constructor.'))
a('metabuilder','src/lib.rs','builder','Metadata','    pub fn builder() -> MetadataBuilder',('MetadataBuilder::new()','Metadata::builder delegates to its builder constructor.'))
a('metabuild','src/lib.rs','build','MetadataBuilder','    pub fn build(&self) -> Metadata',('self.metadata.clone()','Metadata building clones its current value.'))
a('recordlevel','src/lib.rs','level','Record','    pub fn level(&self) -> Level',('self.metadata.level()','Record level is obtained from its metadata.'))
a('metalevel','src/lib.rs','level','Metadata','    pub fn level(&self) -> Level',('self.level','Metadata returns its stored level.'), occurrence=1)
a('recordtarget','src/lib.rs','target','Record','    pub fn target(&self) ->',('self.metadata.target()','Record target is obtained from its metadata.'))
a('metatarget','src/lib.rs','target','Metadata','    pub fn target(&self) ->',('self.target','Metadata returns its stored target.'), occurrence=1)
a('maybeget','src/lib.rs','get','MaybeStaticStr','    fn get(&self)',('MaybeStaticStr::Static(s) => s,','Static and borrowed strings can be read through the wrapper.'))
a('file','src/lib.rs','file','Record','    pub fn file(&self) ->',('self.file.map(|s| s.get())','Record file access reads the optional stored string.'))
a('fromusize','src/lib.rs','from_usize','Level','    fn from_usize(u:',('1 => Some(Level::Error),','Integer discriminants map to enabled levels.'))
a('recorddefault','src/lib.rs','default','RecordBuilder','    fn default() -> Self',('Self::new()','Default construction delegates to the record builder constructor.'))
a('setlogger','src/lib.rs','set_logger','','pub fn set_logger(',('set_logger_inner(|| logger)','The public logger setter delegates to one-time initialization.'))
b('parse','Where are logging level names parsed without case sensitivity and Off rejected?', 'parse')
b('off','Where does converting an Off filter produce no enabled logging level?', 'filter')
b('iterate','Where is the ordered sequence of enabled log levels produced?', 'iter')
b('saturation','Where do logging-level and level-filter increments clamp at the upper bound?', 'increase','filterincrease')
b('record-defaults','Where are a log record builder and its default metadata initialized?', 'recordnew')
b('static-file','Where are static source file paths retained in a record builder?', 'recordfile')
b('set-filter','Where are the regular and thread-unsafe updates of the global maximum logging level implemented?', 'setmax','setmaxracy')
b('read-filter','Where is the current runtime logging filter read?', 'max')
b('install','Where is the logger installed once and safely published to other threads?', 'installinner')
b('fallback','Where is a no-op logger returned before global logger initialization?', 'logger')
e('exact-build','Find RecordBuilder.build.', 'recordbuild')
e('exact-metadata','Find MetadataBuilder.new.', 'metanew')
d('record-builder-call','Which constructor does Record.builder invoke?', 'recordbuilder','recordnew','RecordBuilder::new()', cluster='record-defaults')
d('install-call','Which initialization helper does set_logger invoke?', 'setlogger','installinner','set_logger_inner(|| logger)', cluster='install')
m('builder-chain','Trace Record.builder through RecordBuilder.new to Metadata.builder.', ['recordbuilder','recordnew','metabuilder'], ['RecordBuilder::new()','metadata: Metadata::builder().build(),'], cluster='record-defaults')
m('default-builder-chain','Trace RecordBuilder.default through RecordBuilder.new to Metadata.builder.', ['recorddefault','recordnew','metabuilder'], ['Self::new()','metadata: Metadata::builder().build(),'], cluster='record-defaults')
u('ambiguous-build','Find build without specifying the builder type.', 'recordbuild','metabuild')
u('ambiguous-level','Find level without specifying whether it belongs to a record, metadata, or their builders.', 'recordlevel','metalevel','recordlevelset','metalevelset')
n('rotation','Where does the built-in log file rotation worker compress old files?', 'The tracked crate is a logging facade. File sinks, rotation workers and archive compression are not implemented by this package.')
n('http-export','Where does the built-in HTTP telemetry exporter retry failed uploads?', 'The package exposes logging traits and global dispatch. Its tracked source has no HTTP telemetry transport or upload retry implementation.')

repo('humantime')
a('strict','src/date.rs','parse_rfc3339','','pub fn parse_rfc3339(',('if b[10] != b\'T\'','Strict parsing requires the T separator and a UTC designator.'))
a('weak','src/date.rs','parse_rfc3339_weak','','pub fn parse_rfc3339_weak(',('if day > mdays || day == 0 {','The day is checked against the selected month.'),('if second == 60 {','Leap-second input is handled explicitly.'),('second = 59;','A leap-second input is clamped to 59.'))
a('fraction','src/duration.rs','parse_fractional_part','Parser','    fn parse_fractional_part(',('denominator.checked_mul(10).ok_or(Error::NumberOverflow)?','Fraction parsing detects decimal denominator overflow.'))
a('unit','src/duration.rs','parse_unit','Parser','    fn parse_unit(',('Unit::Month => (n.mul(2_630_016)?, 0)','Duration months use a fixed 30.44-day conversion.'),('Unit::Nanosecond => return Err(Error::NumberOverflow)','Fractional nanoseconds are not representable.'))
a('exactdiv','src/duration.rs','div','u64','    fn div(self, other: Self) -> Result<Self, Error> {',('match self % other {','Fraction conversion checks for a nonzero remainder.'),('_ => Err(Error::NumberOverflow),','Inexact division reports a precision overflow.'))
a('add','src/duration.rs','add_current','','fn add_current(',('if nsec >= 1_000_000_000 {','Accumulated nanoseconds carry into seconds.'))
a('zero','src/duration.rs','parse_duration','','pub fn parse_duration(',('if s == "0" {','A bare zero is accepted without a unit.'))
a('unitnames','src/duration.rs','from_str','Unit','    fn from_str(s: &str) -> Result<Self, Self::Err>',('"minutes" | "minute" | "min" | "mins" | "m"','Minute spellings map to the minute unit.'))
a('durfmt','src/duration.rs','fmt','FormattedDuration','    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result',('f.write_str("0s")?','Zero duration is displayed as 0s.'))
a('datefmt','src/date.rs','fmt','Rfc3339Timestamp','    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result',('let offset = if self.1 == Seconds || nanos == 0 && self.1 == Smart {','Smart timestamp formatting omits the fractional portion when zero.'))
a('wraptime','src/wrapper.rs','from_str','Timestamp','    fn from_str(s: &str) -> Result<Timestamp, Self::Err>',('parse_rfc3339_weak(s).map(Timestamp)','The Timestamp wrapper uses the permissive parser.'))
a('wrapdur','src/wrapper.rs','from_str','Duration','    fn from_str(s: &str) -> Result<Duration, Self::Err>',('parse_duration(s).map(Duration)','The Duration wrapper delegates to duration parsing.'))
a('leap','src/date.rs','is_leap_year','','fn is_leap_year(',('y % 4 == 0','Leap-year arithmetic checks divisible years.'))
a('millis','src/date.rs','format_rfc3339_millis','','pub fn format_rfc3339_millis(',('Rfc3339Timestamp(system_time, Precision::Millis)','The millisecond formatter selects millisecond precision.'))
a('digits','src/date.rs','two_digits','','fn two_digits(',('two_digits_inner(b1 as char, b2 as char)','Two-byte digit conversion delegates to its checked inner converter.'))
a('digitsinner','src/date.rs','two_digits_inner','two_digits','    fn two_digits_inner(',('Some((a * 10 + b) as u64)','Two decimal digits are combined into a value.'))
a('parser','src/duration.rs','parse','Parser','    fn parse(mut self)',('self.parse_unit(n, frac, start, off, &mut out)?','Parsed duration components are accumulated through parse_unit.'))
a('dateerror','src/date.rs','fmt','Error','    fn fmt(&self, f: &mut fmt::Formatter<',('Error::InvalidFormat =>','Date parse errors have a display implementation.'))
a('durationerror','src/duration.rs','fmt','Error','    fn fmt(&self, f: &mut fmt::Formatter<',('Error::NumberOverflow =>','Duration overflow errors have a display implementation.'))
b('strict','Where are strict RFC3339 separator and timezone requirements checked?', 'strict')
b('calendar','Where are invalid days of a month rejected?', 'weak')
b('leap-second','Where is a leap second converted to the preceding representable second?', 'weak', cluster='calendar')
b('decimal-overflow','Where does parsing a decimal duration check denominator overflow?', 'fraction')
b('month','Where are duration months converted using a fixed number of seconds?', 'unit')
b('fractional-ns','Where are decimal nanosecond units and fractional conversions below integral nanosecond precision rejected?', 'unit','exactdiv', cluster='month')
b('carry','Where are accumulated nanoseconds carried into duration seconds?', 'add')
b('zero','Where can a duration be parsed from zero without a unit suffix?', 'zero')
b('unit-names','Where are different human spellings of minute units recognized?', 'unitnames')
b('duration-zero','Where is a zero-length duration displayed as 0s?', 'durfmt')
e('exact-leap','Find is_leap_year in date.rs.', 'leap')
e('exact-millis','Find format_rfc3339_millis.', 'millis')
d('strict-call','Which permissive parser does parse_rfc3339 call after strict checks?', 'strict','weak','parse_rfc3339_weak(s)', cluster='strict')
d('wrapper-call','Which parser does Timestamp.from_str call?', 'wraptime','weak','parse_rfc3339_weak(s).map(Timestamp)', cluster='calendar')
m('date-chain','Trace parse_rfc3339 through parse_rfc3339_weak to two_digits.', ['strict','weak','digits'], ['parse_rfc3339_weak(s)','let month = two_digits(b[5], b[6])?;'], cluster='strict')
m('duration-chain','Trace parse_duration through Parser.parse to Parser.parse_unit.', ['zero','parser','unit'], ['.parse()','self.parse_unit(n, frac, start, off, &mut out)?'], cluster='month')
u('ambiguous-fromstr','Find from_str without specifying the wrapper type.', 'wraptime','wrapdur')
u('ambiguous-fmt','Find fmt without specifying whether a date or duration error is being displayed.', 'dateerror','durationerror')
n('timezone','Where is an IANA timezone database loaded to apply daylight-saving rules?', 'The tracked timestamp parser supports UTC; it does not implement an IANA timezone database loader or daylight-saving conversion engine.')
n('timer','Where does the background scheduler wake tasks when parsed durations expire?', 'The tracked library parses and formats duration/time values. It has no background task scheduler or timer executor.')

repo('p-queue')
a('concurrency','source/index.ts','#doesConcurrentAllowAnother','PQueue','\tget #doesConcurrentAllowAnother()',('return this.#pending < this.#concurrency;','New tasks are allowed only below the concurrency limit.'))
a('interval','source/index.ts','#doesIntervalAllowAnother','PQueue','\tget #doesIntervalAllowAnother()',('return count < this.#intervalCap;','Interval admission compares the active count with its cap.'))
a('abort','source/index.ts','#onAbort','PQueue','\t#onAbort(',('signal.addEventListener(\'abort\', dispatch, {once: true});','Abort callbacks share a one-shot signal listener.'),('signal.removeEventListener(\'abort\', currentListener.dispatch);','Unused signal listeners are removed.'))
a('enqueue','source/priority-queue.ts','enqueue','PriorityQueue','\tenqueue(',('if (this.#queue.at(-1)!.priority! >= priority) {','Equal and lower priorities append behind the tail.'),('this.#queue.splice(index, 0, element);','Other priorities are inserted into the sorted live range.'))
a('priority','source/priority-queue.ts','setPriority','PriorityQueue','\tsetPriority(',('if (this.#queue[index]!.priority === priority) {','Unchanged priority preserves the task order.'),('this.enqueue(item!.run, {priority, id});','Changed priority removes and re-enqueues the waiting task.'))
a('clear','source/index.ts','clear','PQueue','\tclear(): void',('const error = new DOMException(\'The queue was cleared\', \'AbortError\');','Cleared waiting promises are rejected with AbortError.'),('waitingTask.reject!(error);','All removed waiting tasks are rejected.'))
a('pause','source/index.ts','pause','PQueue','\tpause(): void',('this.#isPaused = true;','Pausing prevents new starts by setting the paused flag.'))
a('idle','source/index.ts','onIdle','PQueue','\tasync onIdle()',('if (this.#pending === 0 && this.#queue.size === 0) {','Idle requires both no queued tasks and no pending tasks.'))
a('backpressure','source/index.ts','onSizeLessThan','PQueue','\tasync onSizeLessThan(',('await this.#onEvent([\'next\', \'active\'], () => this.#queue.size < limit);','Size backpressure listens for completions and starts.'))
a('removefirst','source/priority-queue.ts','#removeFirst','PriorityQueue','\t#removeFirst()',('this.#queue[this.#head] = undefined;','Dequeue releases the consumed task reference.'),('this.#compact();','Large consumed prefixes are compacted.'))
a('compact','source/priority-queue.ts','#compact','PriorityQueue','\t#compact()',('this.#queue.splice(0, this.#head);','Compaction removes the consumed prefix.'))
a('remove','source/priority-queue.ts','remove','PriorityQueue','\tremove(idOrRun:',('this.#removeFirst();','Removing the head uses the shared first-item path.'))
a('dequeue','source/priority-queue.ts','dequeue','PriorityQueue','\tdequeue()',('this.#removeFirst()','Dequeue consumes the first live task.'))
a('filter','source/priority-queue.ts','filter','PriorityQueue','\tfilter(',('if (element.priority === options.priority) {','Filtering selects waiting functions by priority.'))
a('lower','source/lower-bound.ts','lowerBound','','export default function lowerBound',('count = step;','Binary insertion narrows the remaining range.'))
a('process','source/index.ts','#processQueue','PQueue','\t#processQueue()',('while (this.#tryToStartAnother()) {}','Queue processing starts jobs until admission stops.'))
a('try','source/index.ts','#tryToStartAnother','PQueue','\t#tryToStartAnother()',('isStarted = this.#startNextTask();','Admission retries after skipping an aborted task.'))
a('startnext','source/index.ts','#startNextTask','PQueue','\t#startNextTask()',('if (this.#doesIntervalAllowAnother && this.#doesConcurrentAllowAnother) {','Starting requires both interval and concurrency admission.'))
a('publicpriority','source/index.ts','setPriority','PQueue','\tsetPriority(',('this.#queue.setPriority(id, priority);','Public priority updates delegate to the queue implementation.'))
a('size','source/index.ts','size','PQueue','\tget size()',('return this.#queue.size;','Public size exposes the waiting queue size.'))
a('internal-size','source/priority-queue.ts','size','PriorityQueue','\tget size()',('return this.#queue.length - this.#head;','Priority queue size excludes the consumed prefix.'))
b('concurrency','Where is the number of running tasks checked against the concurrency limit?', 'concurrency')
b('rate','Where is the interval task cap checked before more work starts?', 'interval')
b('abort-listeners','Where are abort listeners shared and cleaned up?', 'abort')
b('fifo','Where is FIFO order retained for tasks with equal priority?', 'enqueue')
b('reprioritize','Where is a waiting task moved when its priority changes?', 'priority')
b('clear','Where are waiting task promises rejected when the queue is cleared?', 'clear')
b('pause','Where does pausing stop new task starts?', 'pause')
b('idle','Where does waiting for idle require both an empty queue and no running tasks?', 'idle')
b('backpressure','Where can a producer wait until the waiting queue falls below a size limit?', 'backpressure')
b('memory','Where are dequeued task references dropped and consumed array slots compacted?', 'removefirst')
e('exact-filter','Find PriorityQueue.filter in priority-queue.ts.', 'filter')
e('exact-bound','Find lowerBound in lower-bound.ts.', 'lower')
d('priority-call','Which insertion method is called when PriorityQueue.setPriority changes a priority?', 'priority','enqueue','this.enqueue(item!.run, {priority, id});', cluster='reprioritize')
d('dequeue-call','Which shared removal helper does PriorityQueue.dequeue call?', 'dequeue','removefirst','this.#removeFirst()', cluster='memory')
m('remove-chain','Trace PriorityQueue.remove through first-item removal to prefix compaction.', ['remove','removefirst','compact'], ['this.#removeFirst();','this.#compact();'], cluster='memory')
m('start-chain','Trace PQueue.#processQueue through #tryToStartAnother to #startNextTask.', ['process','try','startnext'], ['while (this.#tryToStartAnother()) {}','isStarted = this.#startNextTask();'])
u('ambiguous-priority','Find setPriority without specifying the owner.', 'priority','publicpriority')
u('ambiguous-size','Find size without specifying the owner.', 'size','internal-size')
n('redis','Where is the Redis-backed durable task queue persisted?', 'This tracked package schedules in-process Promise tasks and has no Redis client or durable queue persistence layer.')
n('workers','Where does the built-in distributed worker manager launch jobs on remote machines?', 'The tracked package manages local async task admission; no distributed worker manager or remote job launcher is implemented.')

repo('superjson')
a('serialize','src/index.ts','serialize','SuperJSON','  serialize(object:',('const output = walker(object, identities, this, this.dedupe);','Serialization traverses values and records identity paths.'),('res.meta.v = 1;','Serialization marks the metadata format version.'))
a('deserialize','src/index.ts','deserialize','SuperJSON','  deserialize<T = unknown>',('let result: T = options?.inPlace ? json : copy(json) as any;','Deserialization copies JSON unless explicitly in place.'))
a('values','src/plainer.ts','applyValueAnnotations','','export function applyValueAnnotations(',('untransformValue(v, type, superJson)','Value annotations restore transformed types.'))
a('equalities','src/plainer.ts','applyReferentialEqualityAnnotations','','export function applyReferentialEqualityAnnotations(',('const object = getDeep(plain, parsePath(path, legacyPaths));','Reference restoration obtains the shared value at its representative path.'),('() => object','Equivalent paths are assigned the same value.'))
a('escape','src/pathstringifier.ts','escapeKey','','export const escapeKey =',('key.replace(/\\\\/g,','Backslashes and dots are escaped in property keys.'))
a('parsepath','src/pathstringifier.ts','parsePath','','export const parsePath =',("if (!legacyPaths && char === '\\\\') {",'Path parsing handles escape characters.'),('result.push(lastSegment);','Path parsing emits the final segment.'))
a('class','src/class-registry.ts','register','ClassRegistry','  register(value: Class',('this.classToAllowedProps.set(value, options.allowProps);','Class registration can retain an allowed-property list.'))
a('custom','src/custom-transformer-registry.ts','findApplicable','CustomTransformerRegistry','  findApplicable<T>',('transformer.isApplicable(v)','Custom transformer selection tests applicability against the value.'))
a('walker','src/plainer.ts','walker','','export const walker =',('if (includes(objectsInThisPath, object)) {','Traversal detects circular references along the active path.'),('transformedValue: null,','Circular or deduplicated objects can use a placeholder.'))
a('validate','src/accessDeep.ts','validatePath','','function validatePath(',('throw new Error(\'__proto__ is not allowed as a property\');','Deep access rejects prototype-polluting property paths.'),('throw new Error(\'constructor is not allowed as a property\');','Constructor path components are also rejected.'))
a('setdeep','src/accessDeep.ts','setDeep','','export const setDeep =',('parent.set(newKey, parent.get(keyToRow));','Map-key updates preserve the associated value.'),('parent.delete(keyToRow);','Changed map keys remove the old key.'))
a('stringify','src/index.ts','stringify','SuperJSON','  stringify(object:',('return JSON.stringify(this.serialize(object));','Stringification serializes before JSON encoding.'))
a('parse','src/index.ts','parse','SuperJSON','  parse<T = unknown>',('return this.deserialize(JSON.parse(string), { inPlace: true });','Parsing decodes JSON and restores values in place.'))
a('getid','src/registry.ts','getIdentifier','Registry','  getIdentifier(',('return this.kv.getByValue(value);','Registry identifiers use reverse lookup.'))
a('getbyvalue','src/double-indexed-kv.ts','getByValue','DoubleIndexedKV','  getByValue(',('return this.valueToKey.get(value);','Reverse lookup returns the key stored for a value.'))
a('reg','src/registry.ts','register','Registry','  register(',('this.kv.set(identifier, value);','Registry stores identifier/value pairs.'))
a('clear','src/registry.ts','clear','Registry','  clear()',('this.kv.clear();','Registry clearing delegates to its bidirectional map.'))
a('kvclear','src/double-indexed-kv.ts','clear','DoubleIndexedKV','  clear()',('this.keyToValue.clear();','Clearing removes forward mappings.'),('this.valueToKey.clear();','Clearing removes reverse mappings.'))
b('serialize','Where are JSON-safe values and versioned type metadata constructed?', 'serialize')
b('copy','Where does deserialization decide whether to copy or mutate the supplied JSON?', 'deserialize')
b('value-types','Where do stored annotations restore non-JSON value types?', 'values')
b('identity','Where are repeated references reattached to the same restored object?', 'equalities')
b('escape','Where are dots and backslashes escaped in annotation paths?', 'escape')
b('classes','Where are allowed properties stored for a registered class?', 'class')
b('custom','Where is an applicable custom transformer selected for a value?', 'custom')
b('cycles','Where does serialization detect circular references on the traversal path?', 'walker')
b('unsafe-paths','Where are prototype-polluting property paths rejected?', 'validate')
b('map-key','Where does changing a map key retain its value and remove the old key?', 'setdeep')
e('exact-path','Find parsePath in pathstringifier.ts.', 'parsepath')
e('exact-reverse','Find DoubleIndexedKV.getByValue.', 'getbyvalue')
d('stringify-call','Which serializer is called by SuperJSON.stringify?', 'stringify','serialize','return JSON.stringify(this.serialize(object));', cluster='serialize')
d('lookup-call','Which reverse lookup does Registry.getIdentifier call?', 'getid','getbyvalue','return this.kv.getByValue(value);')
m('stringify-chain','Trace SuperJSON.stringify through serialize to the value walker.', ['stringify','serialize','walker'], ['return JSON.stringify(this.serialize(object));','const output = walker(object, identities, this, this.dedupe);'], cluster='serialize')
m('parse-chain','Trace SuperJSON.parse through deserialize to restoring value annotations.', ['parse','deserialize','values'], ['return this.deserialize(JSON.parse(string), { inPlace: true });','result = applyValueAnnotations('], cluster='value-types')
u('ambiguous-register','Find register without specifying the registry type.', 'reg','class')
u('ambiguous-clear','Find clear without specifying the registry or map type.', 'clear','kvclear')
n('encryption','Where is JSON encrypted with AES before transmission?', 'The tracked package encodes/restores JSON types and references. It has no AES encryption or transport implementation.')
n('postgres','Where does the built-in PostgreSQL client commit serialized objects?', 'The tracked package serializes values without implementing a PostgreSQL database client or transaction layer.')

# Separate essential facts for distinct facets of one implementation. Shared
# implementations remain a common cluster; their facets are never bootstrap units.
FACT_SELECTION = {
    'go-cache.stale-read': {'getexp': [1]},
    'go-cache.deadline': {'getexp': [0]},
    'marshmallow.partial': {'deserialize': [0]},
    'marshmallow.unknown': {'deserialize': [1, 2]},
    'marshmallow.default': {'fieldload': [0]},
    'marshmallow.field-hooks': {'fieldload': [1, 2]},
    'httprouter.dispatch': {'serve': [0]},
    'httprouter.redirect': {'serve': [1, 2]},
    'humantime.calendar': {'weak': [0]},
    'humantime.leap-second': {'weak': [1, 2]},
    'humantime.month': {'unit': [0]},
    'humantime.fractional-ns': {'unit': [1]},
}
