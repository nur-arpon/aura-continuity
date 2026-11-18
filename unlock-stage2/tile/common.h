//
// Field layout for the "Phone / Watch" tile.
//
// The Microsoft sample demonstrated all nine field types (password box, submit,
// checkbox, combobox, two command links, etc.). We don't want any of that — the
// user never types into this tile. We reduce it to the minimum:
//
//   TILEIMAGE    the icon shown on the tile
//   LARGE_TEXT   the tile caption, "Phone / Watch"
//   LABEL        the small tooltip/label under the logo
//   STATUS_TEXT  "Waiting for phone..." -> "Approved - signing in"
//   SUBMIT_BUTTON hidden until approved; only used when AutoSubmit is OFF
//   PASSWORD     ALWAYS HIDDEN. Filled programmatically from the DPAPI store on
//                approval. Never shown, never typed into.
//
#pragma once
#include "helpers.h"

enum SAMPLE_FIELD_ID
{
    SFI_TILEIMAGE     = 0,
    SFI_LARGE_TEXT    = 1,
    SFI_LABEL         = 2,
    SFI_STATUS_TEXT   = 3,
    SFI_SUBMIT_BUTTON = 4,
    SFI_PASSWORD      = 5,
    SFI_NUM_FIELDS    = 6,   // keep last
};

struct FIELD_STATE_PAIR
{
    CREDENTIAL_PROVIDER_FIELD_STATE cpfs;
    CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE cpfis;
};

// Initial states. Note the submit button and password are hidden to start with.
// The submit button is revealed (via GetFieldState) only after approval, and only
// matters when AutoSubmit is OFF.
static const FIELD_STATE_PAIR s_rgFieldStatePairs[] =
{
    { CPFS_DISPLAY_IN_BOTH,          CPFIS_NONE },   // SFI_TILEIMAGE
    { CPFS_DISPLAY_IN_BOTH,          CPFIS_NONE },   // SFI_LARGE_TEXT
    { CPFS_HIDDEN,                   CPFIS_NONE },   // SFI_LABEL (hidden; caption already shows the name)
    { CPFS_DISPLAY_IN_SELECTED_TILE, CPFIS_NONE },   // SFI_STATUS_TEXT
    { CPFS_HIDDEN,                   CPFIS_NONE },   // SFI_SUBMIT_BUTTON
    { CPFS_HIDDEN,                   CPFIS_NONE },   // SFI_PASSWORD (never shown)
};

static const CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR s_rgCredProvFieldDescriptors[] =
{
    { SFI_TILEIMAGE,     CPFT_TILE_IMAGE,    L"Image",          CPFG_CREDENTIAL_PROVIDER_LOGO  },
    { SFI_LARGE_TEXT,    CPFT_LARGE_TEXT,    L"Phone / Watch"                                   },
    { SFI_LABEL,         CPFT_SMALL_TEXT,    L"Tooltip",        CPFG_CREDENTIAL_PROVIDER_LABEL },
    { SFI_STATUS_TEXT,   CPFT_SMALL_TEXT,    L"Status"                                          },
    { SFI_SUBMIT_BUTTON, CPFT_SUBMIT_BUTTON, L"Sign in"                                         },
    { SFI_PASSWORD,      CPFT_PASSWORD_TEXT, L"Password"                                         },
};
