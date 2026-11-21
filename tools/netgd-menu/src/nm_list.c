#include <stdlib.h>
#include <string.h>
#include "nm_list.h"

void nm_list_init(nm_list_t *l)
{
    memset(l, 0, sizeof *l);
}

int nm_list_add(nm_list_t *l, const char *folder, unsigned number)
{
    nm_game_t *g;
    size_t n = strlen(folder);
    if (!n || n > NM_FOLDER_MAX) return -1;
    if (l->count >= NM_MAX_GAMES) { l->truncated = 1; return -1; }
    g = &l->items[l->count];
    memset(g, 0, sizeof *g);
    g->number = (uint16_t)number;
    memcpy(g->folder, folder, n);
    memcpy(g->title, folder, n < NM_TITLE_MAX ? n : NM_TITLE_MAX);
    return l->count++;
}

static int ci_cmp(const char *a, const char *b)
{
    for (;; a++, b++) {
        int x = (unsigned char)*a, y = (unsigned char)*b;
        if (x >= 'A' && x <= 'Z') x += 32;
        if (y >= 'A' && y <= 'Z') y += 32;
        if (x != y) return x < y ? -1 : 1;
        if (!x) return 0;
    }
}

static int cmp(const void *pa, const void *pb)
{
    const nm_game_t *a = pa, *b = pb;
    if (a->number && b->number) {
        if (a->number != b->number) return a->number < b->number ? -1 : 1;
        {
            size_t la = strlen(a->folder), lb = strlen(b->folder);
            if (la != lb) return la < lb ? -1 : 1;      /* fewer digits first */
        }
        return strcmp(a->folder, b->folder);
    }
    if (a->number) return -1;
    if (b->number) return 1;
    {
        int c = ci_cmp(a->folder, b->folder);
        return c ? c : strcmp(a->folder, b->folder);
    }
}

void nm_list_sort(nm_list_t *l)
{
    int o = 0;
    qsort(l->items, (size_t)l->count, sizeof l->items[0], cmp);
    for (int i = 0; i < l->count; i++) {
        if (o && l->items[i].number && l->items[o - 1].number == l->items[i].number) {
            l->items[o - 1].flags |= NM_F_DUPLICATE;
            l->duplicate = l->items[i].number;
            continue;                                   /* the first (fewer digits) stays */
        }
        if (o != i) l->items[o] = l->items[i];
        o++;
    }
    l->count = o;
}

int nm_list_find_folder(const nm_list_t *l, const char *folder)
{
    for (int i = 0; i < l->count; i++) if (!strcmp(l->items[i].folder, folder)) return i;
    return -1;
}

int nm_list_find_number(const nm_list_t *l, unsigned number)
{
    for (int i = 0; i < l->count; i++) if (l->items[i].number == number) return i;
    return -1;
}
