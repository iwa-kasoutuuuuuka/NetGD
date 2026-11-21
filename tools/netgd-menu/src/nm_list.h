/* The game list: a fixed array, sorted (design 6.1). */
#ifndef NM_LIST_H
#define NM_LIST_H
#include "nm.h"

typedef struct {
    nm_game_t items[NM_MAX_GAMES];
    int count;
    int truncated;              /* an item did not fit: "list truncated" */
    unsigned duplicate;         /* a number that two folders have (0 = none): "duplicate number NN" */
} nm_list_t;

void nm_list_init(nm_list_t *l);
/* Append a folder.  number: nm_slot_number(folder), 0 for a named folder.  The title starts as the folder name.
   Returns the index, or -1 when the list is full (truncated is set).  Names over NM_FOLDER_MAX are not added. */
int nm_list_add(nm_list_t *l, const char *folder, unsigned number);
/* Numbered folders by number, then named folders by name (ASCII, case-insensitive).  Of two folders with the
   same number the one with fewer digits stays, the other is dropped; the survivor gets NM_F_DUPLICATE and
   `duplicate` is set. */
void nm_list_sort(nm_list_t *l);
/* index of the item with this folder name (exact), or -1 */
int nm_list_find_folder(const nm_list_t *l, const char *folder);
/* index of the first item with this number, or -1 */
int nm_list_find_number(const nm_list_t *l, unsigned number);

#endif
