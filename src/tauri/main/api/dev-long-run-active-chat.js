// @ts-check

/**
 * @param {any} hostWindow
 */
function requireSillyTavernContext(hostWindow) {
    const context = hostWindow?.SillyTavern?.getContext?.();
    if (!context || typeof context !== 'object') {
        throw new Error('SillyTavern context is unavailable');
    }
    if (typeof context.generate !== 'function') {
        throw new Error('SillyTavern context generate() is unavailable');
    }
    return context;
}

/**
 * @param {any} context
 */
function isActiveChatReady(context) {
    const chatId = String(context?.chatId ?? '').trim();
    if (!chatId) {
        return false;
    }

    if (context.groupId) {
        return true;
    }

    const activeCharacter = Array.isArray(context.characters)
        ? context.characters[Number(context.characterId)]
        : null;
    return Boolean(String(activeCharacter?.avatar ?? activeCharacter?.name ?? '').trim());
}

/**
 * @param {string} value
 */
function stripExtension(value) {
    return value.replace(/\.[^/.]+$/, '');
}

/**
 * @param {any[]} characters
 * @param {string} activeCharacter
 */
function findCharacterIndexByActiveKey(characters, activeCharacter) {
    const key = String(activeCharacter || '').trim();
    if (!key) {
        return -1;
    }

    const keyStem = stripExtension(key);
    return characters.findIndex((character) => {
        const avatar = String(character?.avatar ?? '').trim();
        const name = String(character?.name ?? '').trim();
        return avatar === key
            || stripExtension(avatar) === keyStem
            || name === key
            || name === keyStem;
    });
}

/**
 * @param {any} context
 * @param {{
 *   activeCharacter?: string;
 *   activeGroup?: string | null;
 *   persistedSelectionError?: string;
 * }} [selection]
 */
function formatActiveChatRestoreDiagnostics(context, selection = {}) {
    const activeCharacter = String(selection.activeCharacter ?? context?.activeCharacter ?? '').trim();
    const activeGroup = String(selection.activeGroup ?? context?.activeGroup ?? '').trim();
    const characters = Array.isArray(context?.characters) ? context.characters : [];
    const matchingIndex = findCharacterIndexByActiveKey(characters, activeCharacter);
    const sampleAvatars = characters
        .slice(0, 5)
        .map((character) => String(character?.avatar ?? character?.name ?? '').trim())
        .filter(Boolean)
        .join(', ');

    return [
        `activeCharacter=${JSON.stringify(activeCharacter)}`,
        `activeGroup=${JSON.stringify(activeGroup)}`,
        `persistedSelectionError=${JSON.stringify(selection.persistedSelectionError ?? '')}`,
        `characterId=${JSON.stringify(context?.characterId ?? null)}`,
        `chatId=${JSON.stringify(String(context?.chatId ?? '').trim())}`,
        `characterCount=${characters.length}`,
        `matchingIndex=${matchingIndex}`,
        `sampleCharacters=${JSON.stringify(sampleAvatars)}`,
    ].join(' ');
}

/**
 * @param {((command: string, args?: any) => Promise<any>) | undefined} safeInvoke
 */
async function readPersistedActiveSelection(safeInvoke) {
    if (typeof safeInvoke !== 'function') {
        return {
            activeCharacter: '',
            activeGroup: '',
        };
    }

    try {
        const response = await safeInvoke('get_sillytavern_settings');
        const settings = typeof response?.settings === 'string'
            ? JSON.parse(response.settings)
            : response;
        return {
            activeCharacter: String(settings?.active_character ?? '').trim(),
            activeGroup: String(settings?.active_group ?? '').trim(),
            persistedSelectionError: '',
        };
    } catch (error) {
        return {
            activeCharacter: '',
            activeGroup: '',
            persistedSelectionError: error instanceof Error ? error.message : String(error),
        };
    }
}

/**
 * @param {any} context
 * @param {((command: string, args?: any) => Promise<any>) | undefined} safeInvoke
 */
async function resolveActiveSelection(context, safeInvoke) {
    const activeCharacter = String(context?.activeCharacter ?? '').trim();
    const activeGroup = String(context?.activeGroup ?? '').trim();
    if (activeCharacter || activeGroup) {
        return {
            activeCharacter,
            activeGroup,
            persistedSelectionError: '',
        };
    }

    return readPersistedActiveSelection(safeInvoke);
}

/**
 * @param {any} hostWindow
 * @param {((command: string, args?: any) => Promise<any>) | undefined} safeInvoke
 */
export async function ensureActiveChatReady(hostWindow, safeInvoke) {
    let context = requireSillyTavernContext(hostWindow);
    if (isActiveChatReady(context)) {
        return context;
    }

    if (typeof context.getCharacters === 'function') {
        await context.getCharacters();
        context = requireSillyTavernContext(hostWindow);
    }

    if (isActiveChatReady(context)) {
        return context;
    }

    const selection = await resolveActiveSelection(context, safeInvoke);
    const activeCharacter = selection.activeCharacter;
    const characters = Array.isArray(context.characters) ? context.characters : [];
    const characterIndex = findCharacterIndexByActiveKey(characters, activeCharacter);
    if (characterIndex >= 0 && typeof context.selectCharacterById === 'function') {
        await context.selectCharacterById(characterIndex, { switchMenu: false });
        context = requireSillyTavernContext(hostWindow);
    }

    if (isActiveChatReady(context)) {
        return context;
    }

    if (selection.activeGroup) {
        throw new Error(`Unable to restore active group chat before dev.longRun ${formatActiveChatRestoreDiagnostics(context, selection)}`);
    }

    throw new Error(`Unable to restore active character chat before dev.longRun ${formatActiveChatRestoreDiagnostics(context, selection)}`);
}
