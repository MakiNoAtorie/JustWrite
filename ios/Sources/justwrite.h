// The functions of src/mobile.rs. Strings are UTF-8. Every answer is a JSON string,
// {"ok": …} or {"error": "…"}, that must go back to jw_free.
char *jw_connect(const char *data, const char *server, const char *user, const char *password);
char *jw_projects(const char *data);
char *jw_open(const char *data, const char *name);
char *jw_tree(void);
char *jw_read(const char *id);
char *jw_write(const char *id, const char *text);
char *jw_add_chapter(const char *title);
char *jw_rename(const char *id, const char *title);
char *jw_sync(void);
char *jw_close(void);
void jw_free(char *reply);
