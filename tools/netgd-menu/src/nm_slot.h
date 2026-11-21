/* Numbered image folders: the same rule as common/discimg/src/slot.rs number_of. */
#ifndef NM_SLOT_H
#define NM_SLOT_H

/* The slot number a folder name stands for: 2 to 4 ASCII digits with a value of at least 1
   ("01" -> 1, "002" -> 2, "1000" -> 1000).  0 = not a numbered folder ("1", "00", "12345", "0a", "02 X"). */
unsigned nm_slot_number(const char *name);

#endif
