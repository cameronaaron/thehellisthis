// Production-grade frontend for real-time chat
// Features: proper error handling, accessibility, mobile support, disconnect recovery

// ===================================================================
// Emoji
// ===================================================================

/// The emoji offered by the picker, grouped for reading.
///
/// Emoji typed into a message are ordinary text: they travel through the same
/// Markdown-to-sanitised-HTML pipeline as every other character, so the server
/// needs no list for them and this one is the client's own business.
///
/// Reactions are different — those are server-stored state drawn from a closed
/// set (`REACTION_EMOJI` in `src/emoji.rs`), and `QUICK_REACTIONS` below must
/// stay a subset of it. `client_reaction_roster_matches_the_server` fails if it
/// drifts, because a reaction button the server rejects is a button that
/// silently does nothing.
const EMOJI_GROUPS = [
    {
        id: 'smileys',
        label: 'Smileys & people',
        icon: '😀',
        emoji: ['😀','😃','😄','😁','😆','😅','🤣','😂','🙂','🙃','🫠','😉','😊','😇','🥰','😍','🤩','😘','😗','😚','😙','🥲','😋','😛','😜','🤪','😝','🤑','🤗','🤭','🫢','🤫','🤔','🫡','🤐','🤨','😐','😑','😶','🫥','😏','😒','🙄','😬','🤥','😌','😔','😪','🤤','😴','😷','🤒','🤕','🤢','🤮','🤧','🥵','🥶','🥴','😵','🤯','🤠','🥳','🥸','😎','🤓','🧐','😕','🫤','😟','🙁','😮','😯','😲','😳','🥺','🥹','😦','😧','😨','😰','😥','😢','😭','😱','😖','😣','😞','😓','😩','😫','🥱','😤','😡','😠','🤬','😈','👿','💀','💩','🤡','👻','👽','🤖','🎃'],
    },
    {
        id: 'gestures',
        label: 'Gestures & body',
        icon: '👋',
        emoji: ['👋','🤚','🖐️','✋','🖖','🫱','🫲','🫳','🫴','👌','🤌','🤏','✌️','🤞','🫰','🤟','🤘','🤙','👈','👉','👆','🖕','👇','☝️','🫵','👍','👎','✊','👊','🤛','🤜','👏','🙌','🫶','👐','🤲','🤝','🙏','✍️','💅','🤳','💪','🦾','🦵','🦶','👂','👃','🧠','🫀','🫁','🦷','👀','👁️','👅','👄','🫦'],
    },
    {
        id: 'animals',
        label: 'Animals & nature',
        icon: '🦊',
        emoji: ['🐶','🐱','🐭','🐹','🐰','🦊','🐻','🐼','🐨','🐯','🦁','🐮','🐷','🐸','🐵','🙈','🙉','🙊','🐔','🐧','🐦','🐤','🦆','🦅','🦉','🦇','🐺','🐗','🐴','🦄','🐝','🪱','🐛','🦋','🐌','🐞','🐜','🪰','🦂','🐢','🐍','🦎','🦖','🐙','🦑','🦐','🦞','🦀','🐡','🐠','🐟','🐬','🐳','🐋','🦈','🐊','🐅','🦓','🦍','🦧','🐘','🦛','🐪','🦒','🦘','🐃','🐂','🐄','🐎','🐖','🐏','🐑','🦙','🐐','🦌','🐕','🐩','🦮','🐈','🐓','🦃','🦤','🦚','🦜','🦢','🦩','🕊️','🐇','🦝','🦨','🦡','🦫','🦦','🦥','🐁','🐀','🐿️','🦔','🌵','🎄','🌲','🌳','🌴','🌱','🌿','☘️','🍀','🎍','🪴','🎋','🍃','🍂','🍁','🍄','🐚','🪨','🌾','💐','🌷','🌹','🥀','🌺','🌸','🌼','🌻','🌞','🌝','🌛','🌜','🌚','🌕','🌙','⭐','🌟','✨','⚡','☄️','💥','🔥','🌪️','🌈','☀️','🌤️','⛅','☁️','🌧️','⛈️','🌩️','🌨️','❄️','☃️','⛄','💨','💧','💦','🌊'],
    },
    {
        id: 'food',
        label: 'Food & drink',
        icon: '🍕',
        emoji: ['🍏','🍎','🍐','🍊','🍋','🍌','🍉','🍇','🍓','🫐','🍈','🍒','🍑','🥭','🍍','🥥','🥝','🍅','🍆','🥑','🥦','🥬','🥒','🌶️','🫑','🌽','🥕','🫒','🧄','🧅','🥔','🍠','🥐','🥯','🍞','🥖','🥨','🧀','🥚','🍳','🧈','🥞','🧇','🥓','🥩','🍗','🍖','🌭','🍔','🍟','🍕','🫓','🥪','🥙','🧆','🌮','🌯','🫔','🥗','🥘','🫕','🥫','🍝','🍜','🍲','🍛','🍣','🍱','🥟','🦪','🍤','🍙','🍚','🍘','🍥','🥠','🥮','🍢','🍡','🍧','🍨','🍦','🥧','🧁','🍰','🎂','🍮','🍭','🍬','🍫','🍿','🍩','🍪','🌰','🥜','🍯','🥛','🍼','🫖','☕','🍵','🧃','🥤','🧋','🍶','🍺','🍻','🥂','🍷','🥃','🍸','🍹','🧉','🍾','🧊'],
    },
    {
        id: 'activity',
        label: 'Activity & travel',
        icon: '⚽',
        emoji: ['⚽','🏀','🏈','⚾','🥎','🎾','🏐','🏉','🥏','🎱','🪀','🏓','🏸','🏒','🏑','🥍','🏏','🪃','🥅','⛳','🪁','🏹','🎣','🤿','🥊','🥋','🎽','🛹','🛼','🛷','⛸️','🥌','🎿','⛷️','🏂','🪂','🏋️','🤼','🤸','⛹️','🤺','🤾','🏌️','🏇','🧘','🏄','🏊','🤽','🚣','🧗','🚴','🚵','🎪','🎭','🎨','🎬','🎤','🎧','🎼','🎹','🥁','🪘','🎷','🎺','🪗','🎸','🪕','🎻','🎲','♟️','🎯','🎳','🎮','🎰','🧩','🚗','🚕','🚙','🚌','🚎','🏎️','🚓','🚑','🚒','🚐','🛻','🚚','🚛','🚜','🦯','🦽','🦼','🛴','🚲','🛵','🏍️','🛺','🚨','🚔','🚍','🚘','🚖','🚡','🚠','🚟','🚃','🚋','🚞','🚝','🚄','🚅','🚈','🚂','🚆','🚇','🚊','🚉','✈️','🛫','🛬','🛩️','💺','🛰️','🚀','🛸','🚁','🛶','⛵','🚤','🛥️','🛳️','⛴️','🚢','⚓','🪝','⛽','🚧','🚦','🚥','🗺️','🗿','🗽','🗼','🏰','🏯','🏟️','🎡','🎢','🎠','⛲','⛱️','🏖️','🏝️','🏜️','🌋','⛰️','🏔️','🗻','🏕️','⛺','🏠','🏡','🏘️','🏚️','🏗️','🏭','🏢','🏬','🏣','🏤','🏥','🏦','🏨','🏪','🏫','🏩','💒','🏛️','⛪','🕌','🕍','🛕','🕋','⛩️','🌁','🌃','🏙️','🌄','🌅','🌆','🌇','🌉'],
    },
    {
        id: 'objects',
        label: 'Objects',
        icon: '💡',
        emoji: ['⌚','📱','💻','⌨️','🖥️','🖨️','🖱️','🕹️','💽','💾','💿','📀','📼','📷','📸','📹','🎥','📽️','📞','☎️','📟','📠','📺','📻','🎙️','⏱️','⏲️','⏰','🕰️','⌛','⏳','📡','🔋','🔌','💡','🔦','🕯️','🪔','🧯','🛢️','💸','💵','💴','💶','💷','🪙','💰','💳','💎','⚖️','🪜','🧰','🪛','🔧','🔨','⚒️','🛠️','⛏️','🪚','🔩','⚙️','🪤','🧱','⛓️','🧲','🔫','💣','🧨','🪓','🔪','🗡️','⚔️','🛡️','🚬','⚰️','🪦','⚱️','🏺','🔮','📿','🧿','💈','⚗️','🔭','🔬','🕳️','🩹','🩺','💊','💉','🩸','🧬','🦠','🧫','🧪','🌡️','🧹','🪣','🧼','🪥','🧽','🧴','🔑','🗝️','🚪','🪑','🛋️','🛏️','🛌','🧸','🪆','🖼️','🪞','🪟','🛍️','🛒','🎁','🎈','🎏','🎀','🪄','🪅','🎊','🎉','🎎','🏮','🎐','🧧','✉️','📩','📨','📧','💌','📥','📤','📦','🏷️','🪧','📪','📫','📬','📭','📮','📯','📜','📃','📄','📑','🧾','📊','📈','📉','🗒️','🗓️','📆','📅','🗑️','📇','🗃️','🗳️','🗄️','📋','📁','📂','🗂️','🗞️','📰','📓','📔','📒','📕','📗','📘','📙','📚','📖','🔖','🧷','🔗','📎','🖇️','📐','📏','🧮','📌','📍','✂️','🖊️','🖋️','✒️','🖌️','🖍️','📝','✏️','🔍','🔎','🔏','🔐','🔒','🔓'],
    },
    {
        id: 'symbols',
        label: 'Symbols',
        icon: '❤️',
        emoji: ['❤️','🧡','💛','💚','💙','💜','🖤','🤍','🤎','💔','❣️','💕','💞','💓','💗','💖','💘','💝','💟','☮️','✝️','☪️','🕉️','☸️','✡️','🔯','🕎','☯️','☦️','🛐','⛎','♈','♉','♊','♋','♌','♍','♎','♏','♐','♑','♒','♓','🆔','⚛️','🉑','☢️','☣️','📴','📳','🈶','🈚','🈸','🈺','🈷️','✴️','🆚','💮','🉐','㊙️','㊗️','🈴','🈵','🈹','🈲','🅰️','🅱️','🆎','🆑','🅾️','🆘','❌','⭕','🛑','⛔','📛','🚫','💯','💢','♨️','🚷','🚯','🚳','🚱','🔞','📵','🚭','❗','❕','❓','❔','‼️','⁉️','🔅','🔆','〽️','⚠️','🚸','🔱','⚜️','🔰','♻️','✅','🈯','💹','❇️','✳️','❎','🌐','💠','Ⓜ️','🌀','💤','🏧','🚾','♿','🅿️','🛗','🈳','🈂️','🛂','🛃','🛄','🛅','🚹','🚺','🚼','⚧️','🚻','🚮','🎦','📶','🈁','🔣','ℹ️','🔤','🔡','🔠','🆖','🆗','🆙','🆒','🆕','🆓','0️⃣','1️⃣','2️⃣','3️⃣','4️⃣','5️⃣','6️⃣','7️⃣','8️⃣','9️⃣','🔟','🔢','#️⃣','*️⃣','⏏️','▶️','⏸️','⏯️','⏹️','⏺️','⏭️','⏮️','⏩','⏪','🔀','🔁','🔂','◀️','🔼','🔽','⏫','⏬','➡️','⬅️','⬆️','⬇️','↗️','↘️','↙️','↖️','↕️','↔️','↪️','↩️','⤴️','⤵️','🔃','🔄','🔚','🔙','🔛','🔝','🔜','✔️','☑️','🔘','⚪','⚫','🔴','🔵','🟠','🟡','🟢','🟣','🟤','🔺','🔻','🔸','🔹','🔶','🔷','🔳','🔲','▪️','▫️','◾','◽','◼️','◻️','⬛','⬜','🟥','🟧','🟨','🟩','🟦','🟪','🟫','🔈','🔇','🔉','🔊','🔔','🔕','📣','📢','👁‍🗨','💬','💭','🗯️','♠️','♣️','♥️','♦️','🃏','🎴','🀄','🕐','🕑','🕒','🕓','🕔','🕕','🕖','🕗','🕘','🕙','🕚','🕛'],
    },
];

/// The emoji offered on the hover bar, in the order people reach for them.
///
/// A subset of the server's `REACTION_EMOJI`, deliberately short: the bar is
/// one click, and a bar with thirty buttons is a menu.
const QUICK_REACTIONS = ['👍', '❤️', '😂', '🎉', '😮', '😢', '🔥', '🙏'];

/// Every emoji the server will accept as a reaction, for the "more" picker.
/// Mirrors `REACTION_EMOJI` in `src/emoji.rs`; the parity test compares them as
/// sets, since the server keeps its copy sorted for binary search and that is
/// not an order to show anybody.
const REACTION_EMOJI = [
    '‼️', '✅', '❌', '❤️', '⭐', '🎉', '🎯', '👀', '👇', '👋', '👍', '👎', '💀', '💜', '💡', '💯',
    '🔥', '😀', '😂', '😅', '😍', '😎', '😐', '😔', '😡', '😢', '😭', '😮', '😱', '😴', '🙄', '🙏',
    '🚀', '🤔', '🤝', '🤣', '🥳', '🫡',
];

/// The server's ceiling on an encoded attachment, mirrored here so the client
/// can re-encode until it fits instead of sending something that will bounce.
///
/// Must equal `MAX_ATTACHMENT_BYTES` in `src/config.rs`; the client shrinking
/// to a larger figure than the server accepts means every photo is rejected
/// after the user has waited for it to encode.
/// `client_attachment_ceiling_matches_the_server` fails if they drift.
const MAX_ATTACHMENT_BYTES = 131072;

/// The close code the server uses when it removes a user for being quiet.
///
/// Must equal `IDLE_CLOSE_CODE` in `src/config.rs`. Without the agreement the
/// client cannot tell an eviction from a dropped connection and reconnects into
/// the room it was just removed from, which is what stopped rooms fading.
const IDLE_CLOSE_CODE = 4001;

/// The close code the server uses when a newer connection under the same
/// identity — typically a second tab — has taken this one's place.
///
/// Must equal `SUPERSEDED_CLOSE_CODE` in `src/config.rs`. Distinct from
/// `IDLE_CLOSE_CODE` for the same reason that one exists at all: a bare close
/// is indistinguishable from a dropped connection, and this tab's own
/// reconnect logic would otherwise steal the identity straight back from the
/// tab that just reclaimed it, flapping the two against each other forever.
const SUPERSEDED_CLOSE_CODE = 4002;

/// How long the reaction bar survives the pointer leaving the message.
///
/// The bar is `position: fixed`, so travelling to it means leaving the message
/// — and, briefly, the chat. Without this grace period the buttons vanished on
/// the way to them and could not be clicked at all with a mouse.
const REACTION_BAR_GRACE_MS = 400;

/// How far the bar sits *over* the message rather than above it, in pixels.
/// A gap is a strip the pointer has to cross, and crossing it reads as leaving.
const REACTION_BAR_OVERLAP = 8;

/// How far apart two messages from one person can be and still be drawn as a
/// single run. A burst then reads as one utterance rather than a stack of
/// separate cards, which is what iMessage does and why it feels continuous.
const GROUPING_WINDOW_MS = 60000;

/// The name of the one room that never gets deleted, only faded. Must match
/// `MAIN_ROOM` in config.rs.
const MAIN_ROOM_NAME = 'main';

/// How long `main` can sit with nobody typing before its history fades, in
/// seconds. Must equal main's fade duration in config.rs (1800 there). This
/// is the one grace period a *present* viewer can genuinely watch run out:
/// `main` fades on room-wide silence whether or not anyone is still
/// connected (constraint #3), so someone reading quietly is not exempt.
const MAIN_ROOM_FADE_SECONDS = 1800;

/// How long *this browser* can go without sending a message before it is
/// disconnected for idling, in seconds. Must equal the backend's idle-message
/// timeout in config.rs (600 there).
///
/// This, not a room-wide clock, is the honest stake in a room you spawned
/// yourself: that room cannot be deleted while anyone is connected to it
/// (constraint #3), so watching room-wide silence and warning "the room is
/// about to disappear" was telling a present viewer something that could not
/// happen to them yet. What can happen to them is losing their own seat for
/// having gone quiet — the same rule that makes the user count mean "people
/// actually here" (§7.1) — so the indicator now counts *their* silence, not
/// the room's.
///
/// Both fixed a real bug: the indicator used to invent its own numbers
/// (10/30/45 seconds) under the belief a room died at 60; it does not, and the
/// warning toast fired at 5% of the real remaining time, every single time a
/// conversation paused to think. §7.3/§7.4: the UI must not lie about the
/// mechanics. A Rust test fails if either constant drifts from its backend
/// value.
const IDLE_EVICTION_SECONDS = 600;

/// Whether the primary pointer is a touchscreen rather than a mouse.
///
/// Used to skip auto-focusing the composer on page load and on an automatic
/// reconnect: on a touchscreen, focusing an input pops the virtual keyboard
/// immediately, before anyone has asked to type anything, and on a short
/// viewport also forces the browser to scroll the focused field into view —
/// which pushed the fixed header off-screen behind it. A native chat app
/// opens the keyboard on your tap, not for you; explicit user actions (reply,
/// send, attach) still focus the composer unconditionally, since the keyboard
/// being up at that point is expected.
function isTouchDevice() {
    return window.matchMedia('(pointer: coarse)').matches;
}

/// Search keywords, so typing "fire" finds 🔥 without shipping a full
/// annotation database. Only the emoji people actually search for by name.
const EMOJI_KEYWORDS = {
    '😀': 'grin smile happy', '😂': 'laugh cry joy lol', '🤣': 'rofl laugh lol',
    '🙂': 'smile', '😉': 'wink', '😍': 'love heart eyes', '🥰': 'love adore',
    '😘': 'kiss', '😎': 'cool sunglasses', '🤔': 'think hmm', '😐': 'neutral meh',
    '🙄': 'eyeroll annoyed', '😢': 'sad cry', '😭': 'sob cry bawl', '😡': 'angry mad rage',
    '😱': 'scream shock fear', '😴': 'sleep tired zzz', '🥳': 'party celebrate',
    '💀': 'skull dead dying', '👻': 'ghost boo', '🤖': 'robot bot ai',
    '👍': 'thumbs up yes approve like', '👎': 'thumbs down no disapprove',
    '👏': 'clap applause', '🙏': 'pray thanks please', '🤝': 'handshake deal agree',
    '💪': 'muscle strong flex', '👀': 'eyes look watch', '👋': 'wave hello hi bye',
    '❤️': 'heart love red', '💔': 'broken heart', '🔥': 'fire lit hot flame',
    '⭐': 'star', '✨': 'sparkle shiny', '⚡': 'lightning bolt zap fast',
    '🎉': 'party tada celebrate congrats', '🎊': 'confetti party', '🎁': 'gift present',
    '✅': 'check tick yes done', '❌': 'cross no wrong', '💯': 'hundred perfect',
    '🚀': 'rocket launch ship fast', '🐛': 'bug insect', '💡': 'idea lightbulb',
    '🍕': 'pizza food', '☕': 'coffee', '🍺': 'beer', '🎂': 'cake birthday',
    '🐶': 'dog puppy', '🐱': 'cat kitten', '🦊': 'fox', '🦄': 'unicorn',
    '🌈': 'rainbow', '☀️': 'sun sunny', '🌙': 'moon night', '❄️': 'snow cold',
    '💻': 'laptop computer code', '📱': 'phone mobile', '🔒': 'lock secure',
    '💰': 'money cash', '🎯': 'target bullseye goal', '🫡': 'salute yes sir',
};

class ChatApp {
    constructor() {
        this.ws = null;
        this.myAnimalName = null;
        this.myUserId = null;
        this.connected = false;
        this.reconnectAttempts = 0;
        this.maxReconnectAttempts = 10;
        this.maxMessages = 500;
        this.typingUsers = new Map();
        this.typingTimeout = null;
        this.isCurrentlyTyping = false;
        this.lastReadMessageId = null;
        this.audioContext = null;
        this.roomName = this.extractRoomName();
        this.shouldAutoScroll = true;
        this.lastScrollTop = 0;
        // True from connect until the server's ReconnectToken frame, which it
        // sends immediately after the last history message. While set, per
        // message scrolling is suppressed entirely and the scroll handler
        // ignores events, so history loads without the view moving.
        this.isLoadingHistory = true;
        this.cleanupIntervalId = null;
        this.scrollTimeoutId = null;
        this.autoScrollFrameId = null;
        this.programmaticScroll = false;
        this.historyLoadTimeoutId = null;
        
        // Room heartbeat tracking
        this.roomStartTime = Date.now();
        this.lastActivityTime = Date.now();
        // When *this* browser last sent a message — distinct from
        // `lastActivityTime`, which is room-wide. A custom room cannot be
        // deleted while anyone is connected (constraint #3), so the real risk
        // to a present, silent viewer there is their own idle eviction, keyed
        // to their own last send, not to how quiet the room around them is.
        this.myLastSentTime = Date.now();
        this.lastMessageTime = 0;
        this.recentMessageCount = 0;
        this.lastSenderId = null;
        this.lastSentAt = 0;
        this.heartbeatIntervalId = null;
        this.fadeWarningShown = false;
        
        // Heartbeat monitoring - detect server silence
        this.lastHeartbeatTime = Date.now();
        this.heartbeatTimeoutId = null;
        
        // Sound mute state (persisted in localStorage)
        this.isMuted = localStorage.getItem('chatMuted') === 'true';
        
        // Reply state
        this.replyingTo = null; // { messageId, authorName, text }

        // Composer extras
        this.pendingAttachment = null;
        this.reactionTarget = null;
        this.reactionBarFor = null;
        this.reactionBarHideId = null;
        // True after either idle eviction or being superseded by another tab
        // under the same identity — both leave the socket closed on purpose,
        // and both are undone only by a deliberate action from the person
        // looking at this tab, never automatically. See handleIdleEviction
        // and handleSuperseded for why each one gets here.
        this.awaitingManualRejoin = false;
        this.roster = [];
        this.unreadCount = 0;
        this.dragDepth = 0;
        this.lightboxReturnFocus = null;
        this.lastRenderedDay = null;
        
        // DOM elements
        this.chat = document.getElementById('chat');
        this.input = document.getElementById('messageInput');
        this.sendButton = document.getElementById('sendBtn');
        this.userCountNumEl = document.getElementById('userCountNum');
        this.statusChip = document.getElementById('statusChip');
        this.typingIndicator = document.getElementById('typingIndicator');
        this.typingText = document.getElementById('typingText');
        this.statusText = document.getElementById('statusText');
        this.charCount = document.getElementById('charCount');
        this.roomNameEl = document.getElementById('roomName');
        this.welcomeBanner = document.getElementById('welcomeBanner');
        this.createRoomBtn = document.getElementById('createRoomBtn');
        this.roomHeartbeat = document.getElementById('roomHeartbeat');
        this.roomLifespan = document.getElementById('roomLifespan');
        this.muteBtn = document.getElementById('muteBtn');
        this.replyPreview = document.getElementById('replyPreview');
        this.replyPreviewAuthor = document.getElementById('replyPreviewAuthor');
        this.replyPreviewText = document.getElementById('replyPreviewText');
        this.replyPreviewClose = document.getElementById('replyPreviewClose');
        this.attachBtn = document.getElementById('attachBtn');
        this.fileInput = document.getElementById('fileInput');
        this.attachmentTray = document.getElementById('attachmentTray');
        this.attachmentPreview = document.getElementById('attachmentPreview');
        this.attachmentName = document.getElementById('attachmentName');
        this.attachmentSize = document.getElementById('attachmentSize');
        this.attachmentRemove = document.getElementById('attachmentRemove');
        this.emojiBtn = document.getElementById('emojiBtn');
        this.emojiPanel = document.getElementById('emojiPanel');
        this.emojiSearch = document.getElementById('emojiSearch');
        this.emojiTabs = document.getElementById('emojiTabs');
        this.emojiGrid = document.getElementById('emojiGrid');
        this.emojiEmpty = document.getElementById('emojiEmpty');
        this.reactionBar = document.getElementById('reactionBar');
        this.lightbox = document.getElementById('lightbox');
        this.lightboxImage = document.getElementById('lightboxImage');
        this.lightboxClose = document.getElementById('lightboxClose');
        this.jumpLatest = document.getElementById('jumpLatest');
        this.jumpLatestCount = document.getElementById('jumpLatestCount');
        this.dropOverlay = document.getElementById('dropOverlay');
        this.participantsSheet = document.getElementById('participantsSheet');
        this.participantsList = document.getElementById('participantsList');
        this.participantsCount = document.getElementById('participantsCount');
        this.participantsClose = document.getElementById('participantsClose');
        this.roomTitle = document.getElementById('userCount');
        
        this.init();
    }
    
    init() {
        this.setupEventListeners();
        this.setupComposerExtras();
        this.buildEmojiPicker();
        this.initAudioContext();
        this.updateRoomName();
        this.connect();
        
        // Periodic cleanup of stale typing indicators (store ID for cleanup)
        this.cleanupIntervalId = setInterval(() => this.cleanupTypingIndicators(), 1000);
        
        // Start heartbeat monitor
        this.heartbeatIntervalId = setInterval(() => this.updateHeartbeat(), 1000);
        
        // Monitor scroll position to control auto-scroll
        this.chat.addEventListener('scroll', (e) => {
            this.handleChatScroll();
        });
        
        // Cleanup on page unload
        window.addEventListener('beforeunload', () => this.cleanup());
        window.addEventListener('pagehide', () => this.cleanup());
    }
    
    setupEventListeners() {
        this.input.addEventListener('keydown', (e) => this.handleInputKeydown(e));
        this.input.addEventListener('input', () => this.handleInput());
        this.sendButton.addEventListener('click', () => this.sendMessage());
        this.sendButton.addEventListener('touchend', (e) => {
            e.preventDefault();
            this.sendMessage();
        });
        
        // Explore room link. Same class of bug as .nav-back — see
        // withinNavigationCooldown for why this has to survive a page
        // reload, not just guard within one: this also navigates via JS
        // rather than a plain href, so spamming it does not just repeat the
        // same page load, it picks a *different* random room on every click
        // and races each navigation against the last.
        document.getElementById('exploreLink').addEventListener('click', (e) => {
            e.preventDefault();
            if (withinNavigationCooldown()) return;
            startNavigationCooldown();
            const rooms = ['mellow-forest', 'midnight-owl', 'coffee-talks', 'tech-minds', 'random-thoughts', 'chill-zone', 'late-night', 'creative-corner'];
            const room = rooms[Math.floor(Math.random() * rooms.length)];
            window.location.href = `/${room}`;
        });
        
        // Create room button
        this.createRoomBtn.addEventListener('click', () => {
            this.showCreateRoomDialog();
        });
        
        // Share room button
        const shareRoomBtn = document.getElementById('shareRoomBtn');
        if (shareRoomBtn) {
            shareRoomBtn.addEventListener('click', () => {
                this.shareRoom();
            });
        }
        
        // Dismiss banner
        const dismissBtn = document.getElementById('dismissBanner');
        if (dismissBtn) {
            dismissBtn.addEventListener('click', () => {
                this.welcomeBanner.style.display = 'none';
                localStorage.setItem('bannerDismissed', 'true');
            });
        }
        
        // Check if banner was previously dismissed
        if (localStorage.getItem('bannerDismissed') === 'true') {
            this.welcomeBanner.style.display = 'none';
        }
        
        // Mute button
        this.muteBtn.addEventListener('click', () => this.toggleMute());
        this.updateMuteButton();
        
        // Reply preview close
        this.replyPreviewClose.addEventListener('click', () => this.cancelReply());
        
        // Focus input on page load — desktop only; see isTouchDevice.
        if (!isTouchDevice()) this.input.focus();
    }
    
    toggleMute() {
        this.isMuted = !this.isMuted;
        localStorage.setItem('chatMuted', this.isMuted);
        this.updateMuteButton();
        
        // Show feedback
        if (this.isMuted) {
            this.addSystemMessage('Notifications muted 🔇', 'info');
        } else {
            this.addSystemMessage('Notifications unmuted 🔊', 'info');
            // Play a test beep
            this.playNotificationSound(true);
        }
    }
    
    updateMuteButton() {
        // The icons are sprite references, not font ligatures: swap what the
        // <use> points at rather than the element's text.
        const use = this.muteBtn.querySelector('.icon use');
        const muted = this.isMuted;

        this.muteBtn.classList.toggle('muted', muted);
        this.muteBtn.title = muted ? 'Unmute notifications' : 'Mute notifications';
        this.muteBtn.setAttribute(
            'aria-label',
            muted ? 'Unmute notification sounds' : 'Mute notification sounds'
        );
        this.muteBtn.setAttribute('aria-pressed', String(muted));

        if (use) {
            use.setAttribute('href', muted ? '#i-volume-off' : '#i-volume-on');
        }
    }
    
    startReply(messageId, authorName, text) {
        // Strip HTML tags for preview
        const plainText = text.replace(/<[^>]*>/g, '').trim();
        this.replyingTo = { messageId, authorName, text: plainText };
        
        this.replyPreviewAuthor.textContent = `Replying to ${authorName}`;
        this.replyPreviewText.textContent = plainText.length > 100 ? plainText.slice(0, 100) + '...' : plainText;
        this.replyPreview.classList.add('active');
        this.input.focus();
    }
    
    cancelReply() {
        this.replyingTo = null;
        this.replyPreview.classList.remove('active');
    }
    
    scrollToMessage(messageId) {
        const targetMsg = this.chat.querySelector(`[data-message-id="${messageId}"]`);
        if (targetMsg) {
            targetMsg.scrollIntoView({ behavior: 'smooth', block: 'center' });
            targetMsg.classList.add('highlight');
            setTimeout(() => targetMsg.classList.remove('highlight'), 1500);
        }
    }
    
    showCreateRoomDialog() {
        const roomName = prompt('Enter a room name (3-50 characters):\\n\\nAllowed: letters, numbers, hyphens, underscores\\nExample: my-cool-room');
        if (roomName) {
            const trimmed = roomName.trim().toLowerCase();
            if (/^[a-zA-Z0-9][a-zA-Z0-9-_]{1,48}[a-zA-Z0-9]$/.test(trimmed)) {
                window.location.href = `/${trimmed}`;
            } else {
                this.showError('Invalid room name. Use 3-50 characters: letters, numbers, hyphens, underscores.');
            }
        }
    }
    
    shareRoom() {
        const url = window.location.href;
        if (navigator.share) {
            navigator.share({
                title: `Join me in #${this.roomName}`,
                text: 'Join this ephemeral chat room',
                url: url
            }).catch(() => {});
        } else if (navigator.clipboard) {
            navigator.clipboard.writeText(url).then(() => {
                this.addSystemMessage('Room link copied to clipboard!', 'success');
            }).catch(() => {
                this.addSystemMessage('Could not copy link', 'error');
            });
        }
    }
    
    initAudioContext() {
        try {
            const AudioContext = window.AudioContext || window.webkitAudioContext;
            this.audioContext = new AudioContext();
        } catch (e) {
            console.warn('AudioContext not available:', e);
        }
    }
    
    extractRoomName() {
        const pathParts = window.location.pathname.split('/').filter(p => p);
        return pathParts[pathParts.length - 1] || 'main';
    }
    
    updateRoomName() {
        this.roomNameEl.textContent = this.roomName;
    }
    
    connect() {
        const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsURL = `${protocol}//${location.host}/ws/${this.roomName}`;

        // Every connection replays history, reconnections included, so the
        // suppression window reopens for each one.
        this.isLoadingHistory = true;

        // Failsafe: if ReconnectToken never arrives, auto-scroll would stay
        // suppressed for the whole session. Never let a missing frame leave
        // the UI permanently degraded.
        if (this.historyLoadTimeoutId) clearTimeout(this.historyLoadTimeoutId);
        this.historyLoadTimeoutId = setTimeout(() => this.finishHistoryLoad(), 5000);
        
        try {
            this.ws = new WebSocket(wsURL);
            
            this.ws.onopen = () => this.handleWebSocketOpen();
            this.ws.onclose = (e) => this.handleWebSocketClose(e);
            this.ws.onerror = (e) => this.handleWebSocketError(e);
            this.ws.onmessage = (e) => this.handleWebSocketMessage(e);
        } catch (e) {
            this.showError(`Failed to establish connection: ${e.message}`);
            this.updateConnectionStatus('disconnected');
        }
    }
    
    handleWebSocketOpen() {
        console.log('WebSocket connected');
        this.connected = true;
        // Reset activity tracking on connect
        this.lastActivityTime = Date.now();
        this.myLastSentTime = Date.now();
        this.roomStartTime = Date.now();
        this.fadeWarningShown = false;
        if (this.reconnectAttempts > 0) {
            this.addSystemMessage('Reconnected successfully', 'success');
        }
        this.reconnectAttempts = 0;
        this.updateConnectionStatus('connected');
        this.sendButton.disabled = false;
        this.input.disabled = false;
        // Fires on the initial connect and on every automatic reconnect, not
        // just a user-initiated one — desktop only; see isTouchDevice.
        if (!isTouchDevice()) this.input.focus();
    }

    /// Handles the socket closing.
    ///
    /// The close *code* decides whether to come back. Reconnecting is right for
    /// a dropped connection and wrong for an eviction: the server removes a
    /// user who has said nothing for ten minutes so the room can empty and
    /// eventually fade, and a client that reconnects immediately puts them
    /// straight back, so the room never spends a moment empty and never fades.
    /// This handler used to take no argument at all, so it could not tell the
    /// two apart and always reconnected — one open tab kept a room alive
    /// forever.
    handleWebSocketClose(event) {
        console.log('WebSocket disconnected', event && event.code);
        if (this.heartbeatTimeoutId) clearTimeout(this.heartbeatTimeoutId);
        this.connected = false;
        this.updateConnectionStatus('disconnected');
        this.sendButton.disabled = true;
        this.input.disabled = true;

        if (event && event.code === IDLE_CLOSE_CODE) {
            this.handleIdleEviction();
            return;
        }

        if (event && event.code === SUPERSEDED_CLOSE_CODE) {
            this.handleSuperseded();
            return;
        }

        this.tryReconnect();
    }

    /// Stops reconnecting after an idle eviction, and offers the way back.
    ///
    /// Not a dead end: the room is left as it was and any deliberate action —
    /// typing, or pressing the button — rejoins. What it does not do is rejoin
    /// on its own, which is the whole difference between a room that can empty
    /// and one that cannot.
    handleIdleEviction() {
        this.awaitingManualRejoin = true;
        this.reconnectAttempts = 0;
        this.updateConnectionStatus('idle');
        this.addSystemMessage('You went quiet, so the room let you go. Say something to rejoin.', 'info');

        // The input stays usable; using it is what brings the socket back.
        this.input.disabled = false;
        this.sendButton.disabled = false;
    }

    /// Stops reconnecting after another tab reclaimed this identity, and
    /// offers the same manual way back idle eviction does.
    ///
    /// Reconnecting on its own here would immediately take the identity back
    /// from the tab that just reclaimed it — and that tab's own automatic
    /// reconnect would take it right back again, the exact flapping
    /// `SUPERSEDED_CLOSE_CODE` exists to stop. Only a deliberate action from
    /// whoever is actually looking at this tab breaks that cycle.
    handleSuperseded() {
        this.awaitingManualRejoin = true;
        this.reconnectAttempts = 0;
        this.updateConnectionStatus('idle');
        this.addSystemMessage('This room is open in another tab. Say something here to bring it back.', 'info');

        this.input.disabled = false;
        this.sendButton.disabled = false;
    }

    /// Reconnects after an idle eviction or being superseded, on the user's
    /// initiative.
    rejoinAfterIdle() {
        if (!this.awaitingManualRejoin || this.connected) return false;
        this.awaitingManualRejoin = false;
        this.roster = [];
        this.addSystemMessage('Rejoining...', 'info');
        this.connect();
        return true;
    }
    
    handleWebSocketError(error) {
        console.error('WebSocket error:', error);
    }
    
    handleWebSocketMessage(event) {
        try {
            
            // Reset heartbeat timeout on any message (heartbeat + data)
            this.lastHeartbeatTime = Date.now();
            if (this.heartbeatTimeoutId) clearTimeout(this.heartbeatTimeoutId);
            
            // Set timeout for next heartbeat (server sends every 5s, timeout at 8s)
            this.heartbeatTimeoutId = setTimeout(() => {
                console.warn('Heartbeat timeout - server not responding');
                this.connected = false;
                this.updateConnectionStatus('disconnected');
                if (this.ws && this.ws.readyState === WebSocket.OPEN) {
                    this.ws.close();
                }
            }, 8000);
            
            const data = JSON.parse(event.data);
            this.handleServerEvent(data);
        } catch (e) {
            console.error('Failed to parse message:', e);
            this.showError('Received malformed message from server');
        }
    }
    
    tryReconnect() {
        if (this.reconnectAttempts >= this.maxReconnectAttempts) {
            this.showError('Unable to connect. Please refresh the page.');
            return;
        }
        
        const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000);
        this.updateConnectionStatus('connecting');
        
        // Update status text with countdown
        let remaining = Math.ceil(delay / 1000);
        this.statusText.textContent = `Reconnecting in ${remaining}s`;
        
        const countdownInterval = setInterval(() => {
            remaining--;
            if (remaining > 0 && !this.connected) {
                this.statusText.textContent = `Reconnecting in ${remaining}s`;
            } else {
                clearInterval(countdownInterval);
            }
        }, 1000);
        
        setTimeout(() => {
            clearInterval(countdownInterval);
            if (!this.connected && (!this.ws || this.ws.readyState !== WebSocket.CONNECTING)) {
                this.reconnectAttempts++;
                this.statusText.textContent = 'Connecting...';
                this.connect();
            }
        }, delay);
    }
    
    handleServerEvent(data) {
        switch (data.type) {
            case 'Welcome':
                // First frame on every connection: who we are. Everything that
                // distinguishes "my message" from "someone else's" depends on
                // this arriving before history, which the server guarantees.
                this.myUserId = data.user_id;
                this.myAnimalName = data.animal_name;
                break;
            case 'Message':
                this.handleIncomingMessage(data.message);
                break;
            case 'System':
                this.handleSystemEvent(data.event);
                break;
            case 'UserCount':
                this.updateUserCount(data.count);
                break;
            case 'Roster':
                // Only ever arrives because this client asked.
                this.roster = data.users;
                this.renderParticipants();
                break;
            case 'Heartbeat':
                break;
            case 'ReconnectToken':
                // Marks the end of the history replay: the server sends this
                // straight after the last history message. One jump to the
                // bottom here replaces one animation per history frame.
                this.finishHistoryLoad();
                break;
            default:
                console.warn('Unknown event type:', data.type);
        }
    }
    
    handleIncomingMessage(msg) {
        if (!msg || !msg.message_id) {
            console.warn('Invalid message received:', msg);
            return;
        }
        
        // Track activity for heartbeat
        this.lastActivityTime = Date.now();
        this.fadeWarningShown = false;

        // Count what arrives while the reader is scrolled away, so the
        // jump-to-latest button can say how much they are behind. Not counted
        // during the history replay, which is not "new".
        if (!this.isLoadingHistory && !this.isAtBottom() && msg.user_id !== this.myUserId) {
            this.unreadCount += 1;
            this.updateJumpLatest();
        }
        
        this.lastMessageTime = Date.now();

        // Identity comes from the Welcome frame, which the server sends before
        // any history. The cookies are HttpOnly, so document.cookie is empty by
        // design — never reintroduce a read of it here.
        const isSent = Boolean(this.myUserId) && msg.user_id === this.myUserId;

        // Grouping is by *speaker and message time*, not arrival time. It used
        // to be "anything within three seconds of the last render", which meant
        // two people talking at once were drawn as one person's run and — worse
        // — the whole history replay arrived inside one tick, so every message
        // after the first was grouped with the one before it whoever sent it.
        const sentAt = this.parseTimestamp(msg.timestamp);
        const continuesRun =
            msg.user_id === this.lastSenderId &&
            Math.abs(sentAt - this.lastSentAt) < GROUPING_WINDOW_MS;
        this.lastSenderId = msg.user_id;
        this.lastSentAt = sentAt;

        this.renderMessage(msg, isSent, continuesRun);

        // During history replay nothing scrolls; finishHistoryLoad does it once
        // at the end.
        if (!this.isLoadingHistory && this.shouldAutoScroll) {
            this.scrollToBottom();
        }

        // Play notification for received messages
        if (!isSent) {
            this.playNotificationSound();
            
            // Auto-send read receipt if at bottom
            if (this.isAtBottom()) {
                this.sendReadReceipt(msg.message_id);
            }
        }
    }
    
    /// Draws one message.
    ///
    /// The shape is iMessage's, and the reason it is not one flat element is
    /// that only the *bubble* is the coloured, tailed part. The sender's name
    /// sits above it, the avatar beside it, tapbacks overlap its top corner and
    /// the time sits under it — all outside the bubble, none of them tinted by
    /// it. One element carrying the background cannot do that.
    ///
    ///   .message        row: avatar | stack
    ///     .avatar       received only, at the foot of a run
    ///     .stack
    ///       .sender     received only, at the head of a run
    ///       .replied-to
    ///       .bubble     the coloured, tailed part
    ///       .reactions  tapbacks, overlapping the bubble's top corner
    ///       .receipt    the time, under the last of a run
    renderMessage(msg, isSent, continuesRun = false) {
        // Prevent duplicate message renders
        if (this.chat.querySelector(`[data-message-id="${msg.message_id}"]`)) {
            return;
        }

        const sentAt = this.parseTimestamp(msg.timestamp);
        const timestamp = this.formatTimestamp(sentAt);
        this.maybeRenderDateSeparator(sentAt);

        // Only the last bubble of a run carries a tail and an avatar. Adding
        // this one may end the previous one's run, or extend it.
        const previous = this.chat.querySelector('.message:last-of-type');
        if (previous) {
            previous.classList.toggle('run-end', !continuesRun);
        }

        const row = document.createElement('div');
        row.className = `message ${isSent ? 'sent' : 'received'} run-end`;
        row.dataset.messageId = msg.message_id;
        row.dataset.userId = msg.user_id;
        row.setAttribute('role', 'article');
        row.setAttribute('aria-label', `Message from ${msg.animal_name} at ${timestamp}`);

        const stack = document.createElement('div');
        stack.className = 'stack';

        // Received messages are labelled once per run, as in a group chat.
        if (!isSent && !continuesRun) {
            const sender = document.createElement('div');
            sender.className = 'sender';
            sender.textContent = msg.animal_name;
            stack.appendChild(sender);
        }

        if (msg.reply_to) {
            const repliedTo = document.createElement('div');
            repliedTo.className = 'replied-to';
            repliedTo.innerHTML = `
                <div class="replied-to-author">${this.escapeHtml(msg.reply_to.author_name || 'Unknown')}</div>
                <div class="replied-to-text">${this.escapeHtml(msg.reply_to.preview_text || '')}</div>
            `;
            repliedTo.addEventListener('click', () => this.scrollToMessage(msg.reply_to.message_id));
            stack.appendChild(repliedTo);
        }

        const bubble = document.createElement('div');
        bubble.className = 'bubble';
        bubble.title = timestamp;

        if (msg.text.trim()) {
            const content = document.createElement('div');
            content.className = 'message-content';
            content.innerHTML = msg.text; // Already sanitized by server
            bubble.appendChild(content);
        }

        if (msg.attachment) {
            bubble.classList.add('has-image');
            bubble.appendChild(this.renderAttachment(msg.attachment));
        }

        stack.appendChild(bubble);

        // Always present, even when empty: `.reactions:empty` hides it, so a
        // tapback arriving later has somewhere to go without a rebuild.
        const reactions = document.createElement('div');
        reactions.className = 'reactions';
        for (const reaction of msg.reactions || []) {
            reactions.appendChild(
                this.buildReactionPill(msg.message_id, reaction.emoji, reaction.count, reaction.reacted)
            );
        }
        stack.appendChild(reactions);

        const receipt = document.createElement('div');
        receipt.className = 'receipt';
        receipt.textContent = timestamp;
        stack.appendChild(receipt);

        if (!isSent) {
            const avatar = document.createElement('div');
            avatar.className = 'avatar';
            avatar.setAttribute('aria-hidden', 'true');
            avatar.textContent = msg.animal_name[0].toUpperCase();
            row.appendChild(avatar);
        }

        row.appendChild(stack);

        const replyBtn = document.createElement('button');
        replyBtn.className = 'reply-btn';
        replyBtn.setAttribute('aria-label', `Reply to ${msg.animal_name}`);
        replyBtn.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-reply"/></svg>';
        replyBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            this.startReply(msg.message_id, msg.animal_name, msg.text);
        });
        row.appendChild(replyBtn);

        this.chat.appendChild(row);
        this.pruneRenderedMessages();
    }


    /// Wires the attach button, emoji picker, reactions, lightbox and the
    /// paste/drag-and-drop paths.
    ///
    /// All of it is `addEventListener` and none of it is an `onclick=`
    /// attribute, which is what lets the CSP stay `script-src 'self'` with no
    /// `'unsafe-inline'` (constraint #13). An inline handler here would quietly
    /// force the policy open on a server whose job is rendering user Markdown.
    setupComposerExtras() {
        this.attachBtn.addEventListener('click', () => this.fileInput.click());
        this.attachmentRemove.addEventListener('click', () => this.clearAttachment());
        this.fileInput.addEventListener('change', () => {
            const file = this.fileInput.files && this.fileInput.files[0];
            if (file) this.stageAttachment(file);
        });

        this.emojiBtn.addEventListener('click', () => this.toggleEmojiPanel());
        this.emojiSearch.addEventListener('input', () => this.renderEmojiGrid(this.emojiSearch.value));
        this.emojiSearch.addEventListener('keydown', (e) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                this.toggleEmojiPanel(false);
                this.input.focus();
            }
        });

        // Click-away closes the picker, but a click *inside* it must not.
        document.addEventListener('click', (e) => {
            if (!this.emojiPanel.hidden
                && !this.emojiPanel.contains(e.target)
                && !this.emojiBtn.contains(e.target)) {
                this.toggleEmojiPanel(false);
            }
        });

        document.addEventListener('keydown', (e) => {
            if (e.key !== 'Escape') return;
            if (!this.lightbox.hidden) { this.closeLightbox(); return; }
            if (!this.emojiPanel.hidden) { this.toggleEmojiPanel(false); this.input.focus(); return; }
            if (!this.reactionBar.hidden) { this.hideReactionBar(); }
        });

        this.lightboxClose.addEventListener('click', () => this.closeLightbox());
        this.lightbox.addEventListener('click', (e) => {
            if (e.target === this.lightbox) this.closeLightbox();
        });

        // Tapping the room's name opens the list of who is in it, the way
        // tapping a group's name does.
        this.roomTitle.addEventListener('click', () => this.toggleParticipants());
        this.participantsClose.addEventListener('click', () => this.toggleParticipants(false));
        document.addEventListener('click', (e) => {
            if (!this.participantsSheet.hidden
                && !this.participantsSheet.contains(e.target)
                && !this.roomTitle.contains(e.target)) {
                this.toggleParticipants(false);
            }
        });

        this.jumpLatest.addEventListener('click', () => {
            this.shouldAutoScroll = true;
            this.unreadCount = 0;
            this.scrollToBottom();
            this.updateJumpLatest();
        });

        // The reaction bar follows whichever message the pointer is over.
        //
        // Hiding it is deliberately *delayed*, and entering the bar cancels the
        // hide. The bar is `position: fixed` and a sibling of #chat, so moving
        // the pointer from a message towards the bar leaves #chat — which used
        // to hide the bar instantly, on the way to it. The buttons were
        // unreachable with a mouse: they appeared, and vanished the moment you
        // aimed at them.
        this.chat.addEventListener('pointerover', (e) => {
            if (e.pointerType === 'touch') return;
            const message = e.target.closest('.message');
            if (message) {
                this.cancelReactionBarHide();
                this.showReactionBar(message);
            }
        });
        this.chat.addEventListener('pointerleave', (e) => {
            // Moving onto the bar itself is not leaving.
            if (e.relatedTarget && this.reactionBar.contains(e.relatedTarget)) return;
            this.scheduleReactionBarHide();
        });
        this.reactionBar.addEventListener('pointerenter', () => this.cancelReactionBarHide());
        this.reactionBar.addEventListener('pointerleave', () => this.scheduleReactionBarHide());
        // A scroll invalidates the bar's position, and a bar left floating over
        // an unrelated message is worse than no bar.
        this.chat.addEventListener('scroll', () => this.hideReactionBar(), { passive: true });

        // Touch has no hover, so a long press opens the same bar.
        let pressTimer = null;
        this.chat.addEventListener('touchstart', (e) => {
            const message = e.target.closest('.message');
            if (!message) return;
            pressTimer = setTimeout(() => this.showReactionBar(message), 450);
        }, { passive: true });
        for (const event of ['touchend', 'touchmove', 'touchcancel']) {
            this.chat.addEventListener(event, () => {
                if (pressTimer) { clearTimeout(pressTimer); pressTimer = null; }
            }, { passive: true });
        }

        // Paste an image straight into the composer.
        this.input.addEventListener('paste', (e) => {
            const items = e.clipboardData && e.clipboardData.items;
            if (!items) return;
            for (const item of items) {
                if (item.kind === 'file' && item.type.startsWith('image/')) {
                    e.preventDefault();
                    const file = item.getAsFile();
                    if (file) this.stageAttachment(file);
                    return;
                }
            }
        });

        // Drag and drop. `dragDepth` counts enter/leave pairs, because moving
        // over a child element fires `dragleave` on the parent — without the
        // counter the overlay flickers off every time the pointer crosses a
        // message boundary.
        window.addEventListener('dragenter', (e) => {
            if (!this.dragHasFiles(e)) return;
            e.preventDefault();
            this.dragDepth += 1;
            this.dropOverlay.hidden = false;
        });
        window.addEventListener('dragover', (e) => {
            if (this.dragHasFiles(e)) e.preventDefault();
        });
        window.addEventListener('dragleave', (e) => {
            if (!this.dragHasFiles(e)) return;
            this.dragDepth = Math.max(0, this.dragDepth - 1);
            if (this.dragDepth === 0) this.dropOverlay.hidden = true;
        });
        window.addEventListener('drop', (e) => {
            if (!this.dragHasFiles(e)) return;
            e.preventDefault();
            this.dragDepth = 0;
            this.dropOverlay.hidden = true;
            const file = e.dataTransfer.files && e.dataTransfer.files[0];
            if (file) this.stageAttachment(file);
        });
    }

    dragHasFiles(e) {
        return Boolean(e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files'));
    }

    // ===============================================================
    // Images
    // ===============================================================

    /// Turns a picked file into an attachment the server will accept.
    ///
    /// The downscale is not a nicety. A phone photograph is 3–8 MB and the
    /// server's ceiling is 128 KB of base64, because an image lives in the
    /// room's memory rather than in a store — there is no store. Re-encoding
    /// here is what makes "send a photo" work at all, and doing it in a canvas
    /// has the useful side effect of dropping EXIF, so the location the picture
    /// was taken does not travel with it into a room full of strangers.
    async prepareAttachment(file) {
        if (!file || !file.type.startsWith('image/')) {
            this.showError('Only images can be attached');
            return null;
        }

        const bitmap = await this.decodeImage(file);
        if (!bitmap) {
            this.showError('That image could not be read');
            return null;
        }

        // Try progressively harder to get under the ceiling: smaller, then
        // less quality. Each attempt is cheap and the first usually wins.
        for (const maxEdge of [1600, 1280, 1024, 800, 640]) {
            for (const quality of [0.82, 0.7, 0.55]) {
                const encoded = await this.encodeBitmap(bitmap, maxEdge, quality);
                if (encoded && encoded.data.length <= MAX_ATTACHMENT_BYTES) {
                    if (bitmap.close) bitmap.close();
                    return encoded;
                }
            }
        }

        if (bitmap.close) bitmap.close();
        this.showError('That image is too large to send');
        return null;
    }

    /// Decodes a file to something drawable, preferring `createImageBitmap`
    /// and falling back to an `<img>` for browsers without it.
    async decodeImage(file) {
        if (window.createImageBitmap) {
            try {
                return await createImageBitmap(file);
            } catch (e) {
                console.warn('createImageBitmap failed, falling back', e);
            }
        }

        return new Promise((resolve) => {
            const url = URL.createObjectURL(file);
            const img = new Image();
            img.onload = () => { URL.revokeObjectURL(url); resolve(img); };
            img.onerror = () => { URL.revokeObjectURL(url); resolve(null); };
            img.src = url;
        });
    }

    /// Draws the bitmap into a canvas at most `maxEdge` on its long side and
    /// returns the base64 payload, or null if the browser refused to encode.
    async encodeBitmap(bitmap, maxEdge, quality) {
        const sourceW = bitmap.width;
        const sourceH = bitmap.height;
        if (!sourceW || !sourceH) return null;

        const scale = Math.min(1, maxEdge / Math.max(sourceW, sourceH));
        const width = Math.max(1, Math.round(sourceW * scale));
        const height = Math.max(1, Math.round(sourceH * scale));

        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        const ctx = canvas.getContext('2d');
        if (!ctx) return null;
        ctx.drawImage(bitmap, 0, 0, width, height);

        // WebP where available — roughly a third smaller than JPEG at the same
        // quality, which is the difference between sending a photo and being
        // told it is too big. `toDataURL` silently returns PNG for a type the
        // browser cannot encode, so the result is read back rather than assumed.
        let url = canvas.toDataURL('image/webp', quality);
        if (!url.startsWith('data:image/webp')) {
            url = canvas.toDataURL('image/jpeg', quality);
        }

        const match = /^data:([^;,]+);base64,(.*)$/.exec(url);
        if (!match) return null;

        return { mime: match[1], data: match[2], width, height, faded: false };
    }

    /// Stages an image for sending, showing it above the composer.
    async stageAttachment(file) {
        if (this.pendingAttachment) {
            this.showError('One image at a time');
            return;
        }

        this.setAttachmentBusy(true);
        try {
            const attachment = await this.prepareAttachment(file);
            if (!attachment) return;

            this.pendingAttachment = attachment;
            this.attachmentPreview.src = this.attachmentDataUrl(attachment);
            this.attachmentName.textContent = file.name || 'image';
            this.attachmentSize.textContent = this.formatBytes(
                Math.floor(attachment.data.length * 3 / 4)
            );
            this.attachmentTray.hidden = false;
            this.input.focus();
        } finally {
            this.setAttachmentBusy(false);
        }
    }

    clearAttachment() {
        this.pendingAttachment = null;
        this.attachmentTray.hidden = true;
        // Release the preview so the decoded bitmap is not held alive.
        this.attachmentPreview.removeAttribute('src');
        this.fileInput.value = '';
    }

    setAttachmentBusy(busy) {
        this.attachBtn.disabled = busy;
        this.attachBtn.setAttribute('aria-busy', busy ? 'true' : 'false');
    }

    /// Rebuilds the `data:` URL from validated parts.
    ///
    /// The server stores the payload alone and the type it sniffed from the
    /// bytes, never a URL the client wrote — so the scheme and the type here
    /// are the server's answer, not the sender's claim.
    attachmentDataUrl(attachment) {
        return `data:${attachment.mime};base64,${attachment.data}`;
    }

    formatBytes(bytes) {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }

    /// Builds the image element for a message.
    ///
    /// The wrapper gets an `aspect-ratio` from the server's dimensions, so the
    /// space is reserved before the image decodes. Without it every picture
    /// that finishes loading shoves everything below it down the page, which is
    /// the same layout-shift failure as the empty-chat placeholder (§10.3).
    renderAttachment(attachment) {
        const wrap = document.createElement('button');
        wrap.type = 'button';
        wrap.className = 'message-image-wrap';
        wrap.style.aspectRatio = `${attachment.width} / ${attachment.height}`;
        wrap.style.width = `${Math.min(320, attachment.width)}px`;

        if (attachment.faded) {
            wrap.classList.add('faded');
            wrap.disabled = true;
            wrap.style.cursor = 'default';
            const note = document.createElement('div');
            note.className = 'message-image-faded';
            note.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-broken-image"/></svg>';
            const label = document.createElement('span');
            label.textContent = 'Image faded';
            note.appendChild(label);
            wrap.appendChild(note);
            wrap.setAttribute('aria-label', 'An image that has faded from this room');
            return wrap;
        }

        const img = document.createElement('img');
        img.className = 'message-image';
        img.loading = 'lazy';
        img.decoding = 'async';
        img.alt = 'Shared image';
        img.src = this.attachmentDataUrl(attachment);
        wrap.appendChild(img);
        wrap.setAttribute('aria-label', 'Open image full size');
        wrap.addEventListener('click', () => this.openLightbox(img.src));
        return wrap;
    }

    openLightbox(src) {
        this.lightboxImage.src = src;
        this.lightbox.hidden = false;
        this.lightboxReturnFocus = document.activeElement;
        this.lightboxClose.focus();
    }

    closeLightbox() {
        this.lightbox.hidden = true;
        this.lightboxImage.removeAttribute('src');
        if (this.lightboxReturnFocus && this.lightboxReturnFocus.focus) {
            this.lightboxReturnFocus.focus();
        }
    }

    // ===============================================================
    // Reactions
    // ===============================================================

    sendReaction(messageId, emoji) {
        if (!this.connected || this.ws.readyState !== WebSocket.OPEN) return;
        this.ws.send(JSON.stringify({ type: 'React', message_id: messageId, emoji }));
        this.hideReactionBar();
    }

    /// Applies one reaction delta from the server.
    ///
    /// The server sends the bucket's new total rather than "add one", so a
    /// client that missed an earlier delta still lands on the right number
    /// instead of counting its own way to a different one.
    applyReaction({ message_id, user_id, emoji, active, count }) {
        const node = this.chat.querySelector(`[data-message-id="${message_id}"]`);
        if (!node) return;

        const container = node.querySelector('.reactions');
        if (!container) return;

        let pill = container.querySelector(`[data-emoji="${CSS.escape(emoji)}"]`);

        if (count === 0) {
            if (pill) pill.remove();
            return;
        }

        if (!pill) {
            pill = this.buildReactionPill(message_id, emoji, count, false);
            container.appendChild(pill);
        }

        pill.querySelector('.reaction-count').textContent = String(count);

        // Only our own reaction changes whether the pill is highlighted.
        if (user_id === this.myUserId) {
            pill.classList.toggle('mine', active);
            pill.setAttribute('aria-pressed', active ? 'true' : 'false');
        }
    }

    buildReactionPill(messageId, emoji, count, mine) {
        const pill = document.createElement('button');
        pill.type = 'button';
        pill.className = `reaction-pill${mine ? ' mine' : ''}`;
        pill.dataset.emoji = emoji;
        pill.setAttribute('aria-pressed', mine ? 'true' : 'false');
        pill.setAttribute('aria-label', `React with ${emoji}`);

        const face = document.createElement('span');
        face.className = 'reaction-emoji';
        face.textContent = emoji;

        const number = document.createElement('span');
        number.className = 'reaction-count';
        number.textContent = String(count);

        pill.append(face, number);
        pill.addEventListener('click', () => this.sendReaction(messageId, emoji));
        return pill;
    }

    showReactionBar(messageNode) {
        if (!this.connected) return;
        const messageId = messageNode.dataset.messageId;
        if (!messageId) return;

        // Already showing for this message: leave it exactly as it is. Rebuilding
        // on every pointerover replaced the button under the cursor between
        // pointerdown and pointerup, so the click landed on nothing.
        if (this.reactionBarFor === messageId && !this.reactionBar.hidden) return;
        this.reactionBarFor = messageId;

        this.reactionBar.replaceChildren();
        for (const emoji of QUICK_REACTIONS) {
            const button = document.createElement('button');
            button.type = 'button';
            button.textContent = emoji;
            button.setAttribute('aria-label', `React with ${emoji}`);
            button.addEventListener('click', (e) => {
                e.stopPropagation();
                this.sendReaction(messageId, emoji);
            });
            this.reactionBar.appendChild(button);
        }

        const more = document.createElement('button');
        more.type = 'button';
        more.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-add-reaction"/></svg>';
        more.setAttribute('aria-label', 'More reactions');
        more.addEventListener('click', (e) => {
            e.stopPropagation();
            this.openReactionPicker(messageId);
        });
        this.reactionBar.appendChild(more);

        this.reactionBar.hidden = false;

        // Positioned against the viewport, above the message and clamped to
        // stay on screen. Fixed rather than absolute so it never becomes part
        // of a message's box and never changes a row's height.
        const rect = messageNode.getBoundingClientRect();
        const barRect = this.reactionBar.getBoundingClientRect();
        // Overlapping the message by a few pixels rather than floating above it:
        // a gap is a strip of nothing the pointer crosses on its way to the
        // buttons, and crossing it counts as leaving.
        const top = Math.max(8, rect.top - barRect.height + REACTION_BAR_OVERLAP);
        const left = Math.min(
            Math.max(8, rect.left),
            window.innerWidth - barRect.width - 8
        );
        this.reactionBar.style.top = `${top}px`;
        this.reactionBar.style.left = `${left}px`;
    }

    hideReactionBar() {
        this.cancelReactionBarHide();
        this.reactionBar.hidden = true;
        this.reactionBarFor = null;
    }

    /// Hides the bar shortly, unless the pointer arrives on it first.
    ///
    /// The delay is the whole mechanism: it is the time the pointer needs to
    /// travel from the message to the buttons.
    scheduleReactionBarHide() {
        this.cancelReactionBarHide();
        this.reactionBarHideId = setTimeout(() => this.hideReactionBar(), REACTION_BAR_GRACE_MS);
    }

    cancelReactionBarHide() {
        if (this.reactionBarHideId) {
            clearTimeout(this.reactionBarHideId);
            this.reactionBarHideId = null;
        }
    }

    /// Opens the emoji panel showing only what the server accepts as a
    /// reaction, and routes a pick to `React` instead of into the composer.
    ///
    /// The reaction roster is a closed set on the server, so offering the full
    /// typing catalogue here would be offering buttons that do nothing.
    openReactionPicker(messageId) {
        this.reactionTarget = messageId;
        this.hideReactionBar();
        this.toggleEmojiPanel(true);
        this.renderEmojiGrid('');
    }

    // ===============================================================
    // Emoji picker
    // ===============================================================

    buildEmojiPicker() {
        this.emojiTabs.replaceChildren();
        for (const group of EMOJI_GROUPS) {
            const tab = document.createElement('button');
            tab.type = 'button';
            tab.className = 'emoji-tab';
            tab.textContent = group.icon;
            tab.title = group.label;
            tab.setAttribute('role', 'tab');
            tab.setAttribute('aria-label', group.label);
            tab.setAttribute('aria-selected', 'false');
            tab.dataset.group = group.id;
            tab.addEventListener('click', () => this.scrollToEmojiGroup(group.id));
            this.emojiTabs.appendChild(tab);
        }
        this.renderEmojiGrid('');
    }

    /// Fills the grid, filtered by `query`.
    ///
    /// Rebuilding the whole grid on each keystroke is O(catalogue) — about two
    /// thousand nodes — which is fine here precisely because it is not on the
    /// message path (§1.1). The panel is not open while messages are arriving
    /// in any quantity, and a person types a search term a few characters long.
    renderEmojiGrid(query) {
        const needle = query.trim().toLowerCase();
        this.emojiGrid.replaceChildren();
        let shown = 0;

        // Reaction mode: one flat group, drawn from the server's closed set.
        const groups = this.reactionTarget
            ? [{ id: 'reactions', label: 'React with', emoji: REACTION_EMOJI }]
            : EMOJI_GROUPS;
        this.emojiTabs.hidden = Boolean(this.reactionTarget);

        for (const group of groups) {
            const matches = needle
                ? group.emoji.filter((e) => this.emojiMatches(e, needle))
                : group.emoji;
            if (!matches.length) continue;

            const label = document.createElement('div');
            label.className = 'emoji-group-label';
            label.textContent = group.label;
            label.dataset.groupLabel = group.id;
            this.emojiGrid.appendChild(label);

            for (const emoji of matches) {
                const cell = document.createElement('button');
                cell.type = 'button';
                cell.className = 'emoji-cell';
                cell.textContent = emoji;
                cell.setAttribute('role', 'option');
                cell.setAttribute('aria-label', emoji);
                cell.addEventListener('click', () => this.pickEmoji(emoji));
                this.emojiGrid.appendChild(cell);
                shown += 1;
            }
        }

        this.emojiEmpty.hidden = shown > 0;
    }

    emojiMatches(emoji, needle) {
        const keywords = EMOJI_KEYWORDS[emoji];
        return Boolean(keywords && keywords.includes(needle));
    }

    scrollToEmojiGroup(groupId) {
        const label = this.emojiGrid.querySelector(`[data-group-label="${groupId}"]`);
        if (label) label.scrollIntoView({ block: 'start' });
        for (const tab of this.emojiTabs.children) {
            tab.setAttribute('aria-selected', tab.dataset.group === groupId ? 'true' : 'false');
        }
    }

    /// Routes a picked emoji: to the message being reacted to, or to the
    /// composer when the panel was opened normally.
    pickEmoji(emoji) {
        if (this.reactionTarget) {
            this.sendReaction(this.reactionTarget, emoji);
            this.reactionTarget = null;
            this.toggleEmojiPanel(false);
            return;
        }
        this.insertEmoji(emoji);
    }

    /// Inserts an emoji at the caret rather than at the end.
    insertEmoji(emoji) {
        const input = this.input;
        const start = input.selectionStart ?? input.value.length;
        const end = input.selectionEnd ?? input.value.length;

        input.value = input.value.slice(0, start) + emoji + input.value.slice(end);
        const caret = start + emoji.length;
        input.setSelectionRange(caret, caret);
        input.focus();
        this.handleInput();
    }

    toggleEmojiPanel(force) {
        const open = force ?? this.emojiPanel.hidden;
        this.emojiPanel.hidden = !open;
        this.emojiBtn.setAttribute('aria-expanded', open ? 'true' : 'false');

        if (open) {
            this.emojiSearch.value = '';
            this.renderEmojiGrid('');
            this.emojiSearch.focus();
        } else {
            // Leaving the panel leaves reaction mode, or the next open would
            // silently still be aimed at an old message.
            this.reactionTarget = null;
            this.emojiTabs.hidden = false;
        }
    }

    // ===============================================================
    // Jump to latest
    // ===============================================================

    /// Shows the jump button when the reader has scrolled away from the bottom,
    /// with a count of what arrived since.
    updateJumpLatest() {
        const away = !this.isAtBottom();
        this.jumpLatest.hidden = !away;

        if (!away) {
            this.unreadCount = 0;
        }

        this.jumpLatestCount.hidden = this.unreadCount === 0;
        this.jumpLatestCount.textContent = String(Math.min(this.unreadCount, 99));
    }

    /// Adds a "Today" / "Yesterday" / date rule when the day changes.
    ///
    /// Driven by the day of the message being rendered rather than by a running
    /// counter, for the same reason `pruneRenderedMessages` counts the DOM: the
    /// separator belongs to the messages actually on screen, and history is
    /// replayed in order, so the day is always derivable from what is being
    /// drawn (§10.1 — never derive state you can read).
    maybeRenderDateSeparator(sentAt) {
        const day = new Date(sentAt);
        if (Number.isNaN(day.getTime())) return;

        const key = day.toDateString();
        if (key === this.lastRenderedDay) return;
        this.lastRenderedDay = key;

        const today = new Date().toDateString();
        const yesterday = new Date(Date.now() - 86400000).toDateString();

        let label;
        if (key === today) {
            label = 'Today';
        } else if (key === yesterday) {
            label = 'Yesterday';
        } else {
            label = day.toLocaleDateString([], {
                month: 'short',
                day: 'numeric',
                year: day.getFullYear() === new Date().getFullYear() ? undefined : 'numeric',
            });
        }

        const rule = document.createElement('div');
        rule.className = 'date-separator';
        rule.setAttribute('role', 'separator');
        rule.textContent = label;
        this.chat.appendChild(rule);
    }

    /// Trims the oldest rendered messages once there are more than maxMessages.
    ///
    /// Counts the DOM directly rather than maintaining a running total. The
    /// counter this replaces was incremented for system messages too, but those
    /// remove themselves after 8 seconds without ever decrementing it — so on a
    /// busy room the count drifted well above the number of nodes actually
    /// present and began deleting live chat messages that were nowhere near the
    /// limit. Messages vanishing from a conversation is about the worst failure
    /// a chat client has, and it was invisible because the counter, not the
    /// page, was wrong.
    pruneRenderedMessages() {
        const messages = this.chat.querySelectorAll('.message');
        const excess = messages.length - this.maxMessages;

        for (let i = 0; i < excess; i++) {
            messages[i].remove();
        }

        // A separator whose messages have all been pruned is a date heading for
        // nothing. Drop any that now sit at the very top or back-to-back.
        for (const rule of this.chat.querySelectorAll('.date-separator')) {
            const next = rule.nextElementSibling;
            if (!next || next.classList.contains('date-separator')) {
                rule.remove();
            }
        }
    }
    
    handleSystemEvent(event) {
        if (!event) return;
        
        if (event.UserJoined) {
            // Compare against the identity the server gave us rather than
            // assuming the first join we see is our own — with two people
            // connecting at once, that assumption picked the wrong user.
            if (event.UserJoined.user_id === this.myUserId) {
                this.addSystemMessage(`You joined as ${event.UserJoined.animal_name} 🎉`, 'success');
            } else {
                this.addSystemMessage(`${event.UserJoined.animal_name} joined`, 'success');
            }
            this.updateRosterFrom(event.UserJoined.animal_name, true);
        } else if (event.UserLeft) {
            this.typingUsers.delete(event.UserLeft.animal_name);
            this.updateTypingIndicator();
            this.addSystemMessage(`${event.UserLeft.animal_name} left`);
            this.updateRosterFrom(event.UserLeft.animal_name, false);
        } else if (event.Typing) {
            this.handleTypingIndicator(event.Typing.animal_name, event.Typing.is_typing);
        } else if (event.Reaction) {
            this.applyReaction(event.Reaction);
        } else if (event.ReadReceipt) {
            this.updateReadReceipt(event.ReadReceipt.animal_name, event.ReadReceipt.message_id);
        } else if (event.ServerShutdown) {
            this.addSystemMessage(`Server is restarting: ${event.ServerShutdown.reason}`, 'warning');
        }
        
        if (!this.isLoadingHistory && this.shouldAutoScroll) {
            this.scrollToBottom();
        }
    }

    handleTypingIndicator(name, isTyping) {
        if (name === this.myAnimalName) return;
        
        if (isTyping) {
            this.typingUsers.set(name, Date.now());
        } else {
            this.typingUsers.delete(name);
        }
        this.updateTypingIndicator();
    }
    
    cleanupTypingIndicators() {
        const now = Date.now();
        let cleaned = false;
        
        for (const [name, timestamp] of this.typingUsers) {
            if (now - timestamp > 5000) {
                this.typingUsers.delete(name);
                cleaned = true;
            }
        }
        
        if (cleaned) {
            this.updateTypingIndicator();
        }
    }
    
    updateTypingIndicator() {
        const names = Array.from(this.typingUsers.keys());
        
        if (names.length === 0) {
            this.typingIndicator.style.display = 'none';
        } else {
            let text;
            if (names.length === 1) {
                text = `${names[0]} is typing...`;
            } else if (names.length === 2) {
                text = `${names[0]} and ${names[1]} are typing...`;
            } else {
                text = `${names.length} people are typing...`;
            }
            
            this.typingText.textContent = text;
            this.typingIndicator.style.display = 'flex';
        }
    }
    
    updateReadReceipt(animal_name, message_id) {
        if (animal_name === this.myAnimalName) {
            this.lastReadMessageId = message_id;
        }
    }
    
    handleInputKeydown(e) {
        // Any deliberate keystroke after an idle eviction is a request to come
        // back; the message itself is still typed and sent as normal.
        this.rejoinAfterIdle();

        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            this.sendMessage();
        }
    }
    
    /// Reads `this.input` directly rather than `e.target`: `insertEmoji` calls
    /// this after setting the value programmatically, with no event at all, so
    /// an `e.target` read threw on every emoji pick from the composer — after
    /// the character had already been inserted, so it looked like nothing was
    /// wrong until the character counter and typing indicator silently stopped
    /// updating.
    handleInput() {
        const text = this.input.value;
        this.charCount.textContent = text.length;
        
        // Send typing indicator
        if (text.length > 0) {
            this.handleTyping();
        } else {
            this.cancelTyping();
        }
    }
    
    handleTyping() {
        if (!this.isCurrentlyTyping) {
            this.isCurrentlyTyping = true;
            this.sendTypingIndicator(true);
        }
        
        if (this.typingTimeout) clearTimeout(this.typingTimeout);
        this.typingTimeout = setTimeout(() => {
            this.cancelTyping();
        }, 3000);
    }
    
    cancelTyping() {
        if (this.isCurrentlyTyping) {
            this.isCurrentlyTyping = false;
            this.sendTypingIndicator(false);
        }
    }
    
    sendMessage() {
        if (this.rejoinAfterIdle()) {
            // The socket is still opening; the text stays in the box and the
            // next press sends it.
            return;
        }

        const text = this.input.value.trim();

        // An image on its own is a message. The server agrees: text is only
        // required when there is nothing else to carry.
        if (!text && !this.pendingAttachment) {
            this.input.focus();
            return;
        }
        
        if (!this.connected) {
            this.showError('Not connected. Attempting to reconnect...');
            return;
        }
        
        if (text.length > 8000) {
            this.showError('Message is too long (max 8000 characters)');
            return;
        }
        
        try {
            const messagePayload = { type: 'Message', text };

            if (this.pendingAttachment) {
                messagePayload.attachment = this.pendingAttachment;
            }
            
            // Include reply info if replying
            if (this.replyingTo) {
                messagePayload.reply_to = {
                    message_id: this.replyingTo.messageId,
                    author_name: this.replyingTo.authorName,
                    preview_text: this.replyingTo.text.slice(0, 100)
                };
            }
            
            this.ws.send(JSON.stringify(messagePayload));
            this.myLastSentTime = Date.now();
            this.input.value = '';
            this.clearAttachment();
            this.toggleEmojiPanel(false);
            this.charCount.textContent = '0';
            this.isCurrentlyTyping = false;
            this.cancelReply(); // Clear reply state after sending
            if (this.typingTimeout) clearTimeout(this.typingTimeout);
            this.sendTypingIndicator(false);
            this.input.focus();
        } catch (e) {
            console.error('Failed to send message:', e);
            this.showError('Failed to send message. Please try again.');
        }
    }
    
    sendTypingIndicator(is_typing) {
        if (this.connected && this.ws.readyState === WebSocket.OPEN) {
            try {
                this.ws.send(JSON.stringify({ type: 'Typing', is_typing }));
            } catch (e) {
                console.warn('Failed to send typing indicator:', e);
            }
        }
    }
    
    sendReadReceipt(message_id) {
        if (this.connected && message_id && this.ws.readyState === WebSocket.OPEN) {
            try {
                this.ws.send(JSON.stringify({ type: 'ReadReceipt', message_id }));
            } catch (e) {
                console.warn('Failed to send read receipt:', e);
            }
        }
    }
    
    /// Pins the view to the newest message.
    ///
    /// Coalesced into one animation frame: a burst of messages must cost one
    /// scroll write after layout, not one per message. Scrolling from a
    /// `setTimeout(0)` ran before layout had settled, so the target height was
    /// occasionally stale and the view stopped a message short of the bottom.
    scrollToBottom() {
        if (this.autoScrollFrameId) return;

        this.autoScrollFrameId = requestAnimationFrame(() => {
            this.autoScrollFrameId = null;
            // Suppress the scroll handler for the event this write produces:
            // mid-scroll the element is not yet at the bottom, and reading that
            // back would clear shouldAutoScroll and strand the user.
            this.programmaticScroll = true;
            this.chat.scrollTop = this.chat.scrollHeight;
            requestAnimationFrame(() => { this.programmaticScroll = false; });
        });
    }

    /// Called on the ReconnectToken frame, which follows the last history
    /// message.
    finishHistoryLoad() {
        if (this.historyLoadTimeoutId) {
            clearTimeout(this.historyLoadTimeoutId);
            this.historyLoadTimeoutId = null;
        }
        if (!this.isLoadingHistory) return;

        this.isLoadingHistory = false;
        this.shouldAutoScroll = true;
        this.scrollToBottom();
    }

    handleChatScroll() {
        // Ignore scroll events the client caused itself, and everything during
        // history replay. Both would otherwise be read as "the user scrolled
        // up" and would switch auto-scroll off at exactly the wrong moment —
        // the intermittent part of the reload jumping.
        if (this.programmaticScroll || this.isLoadingHistory) return;

        const wasAtBottom = this.shouldAutoScroll;
        this.shouldAutoScroll = this.isAtBottom();
        this.updateJumpLatest();

        // Only send read receipt if user just scrolled to bottom (debounce)
        if (this.shouldAutoScroll && !wasAtBottom) {
            const messages = this.chat.querySelectorAll('.message');
            if (messages.length > 0) {
                const lastMsg = messages[messages.length - 1];
                const messageID = lastMsg.dataset.messageId;
                if (messageID && messageID !== this.lastReadMessageId) {
                    this.sendReadReceipt(messageID);
                }
            }
        }
    }
    
    isAtBottom() {
        return this.chat.scrollHeight - this.chat.scrollTop - this.chat.clientHeight < 50;
    }
    
    updateUserCount(count) {
        this.userCountNumEl.textContent = count;
    }

    /// Opens or closes the list of people in the room.
    ///
    /// The roster is asked for on open rather than pushed with every join:
    /// broadcasting the whole list to everyone whenever anybody arrives is
    /// quadratic in the size of the room, for a panel almost nobody has open.
    /// While it *is* open, the `UserJoined` and `UserLeft` events the client
    /// already receives keep it current.
    toggleParticipants(force) {
        const open = force ?? this.participantsSheet.hidden;
        this.participantsSheet.hidden = !open;
        this.roomTitle.setAttribute('aria-expanded', open ? 'true' : 'false');

        if (open) {
            this.requestRoster();
            this.participantsClose.focus();
        }
    }

    requestRoster() {
        if (!this.connected || this.ws.readyState !== WebSocket.OPEN) return;
        this.ws.send(JSON.stringify({ type: 'RequestRoster' }));
    }

    renderParticipants() {
        this.participantsCount.textContent =
            this.roster.length === 1 ? '1 here' : `${this.roster.length} here`;

        this.participantsList.replaceChildren();
        for (const name of this.roster) {
            const row = document.createElement('li');
            row.className = 'participant';

            const avatar = document.createElement('span');
            avatar.className = 'participant-avatar';
            avatar.setAttribute('aria-hidden', 'true');
            avatar.textContent = name[0].toUpperCase();

            const label = document.createElement('span');
            label.className = 'participant-name';
            label.textContent = name;

            row.append(avatar, label);

            if (name === this.myAnimalName) {
                row.classList.add('is-me');
                const you = document.createElement('span');
                you.className = 'participant-you';
                you.textContent = 'you';
                row.appendChild(you);
            }

            this.participantsList.appendChild(row);
        }
    }

    /// Keeps an open list current without asking again.
    updateRosterFrom(name, arrived) {
        if (this.participantsSheet.hidden) return;

        if (arrived && !this.roster.includes(name)) {
            this.roster = [...this.roster, name].sort();
        } else if (!arrived) {
            this.roster = this.roster.filter((n) => n !== name);
        }
        this.renderParticipants();
    }
    
    /// Always visible: green/connected, amber/connecting, grey/idle,
    /// red/disconnected — the standard traffic-light meaning for each
    /// colour, so the one state that's actually good news is the one state
    /// that used to say nothing at all. It used to hide the chip entirely
    /// while connected, on the reasoning that a healthy connection is not
    /// news — true for a *transition* into it, but it left no way to answer
    /// "is this thing actually connected right now" by looking, only by its
    /// absence.
    updateConnectionStatus(status) {
        this.statusChip.hidden = false;
        this.statusChip.className = `status-chip ${status}`;

        switch (status) {
            case 'connected':
                this.statusText.textContent = 'Connected';
                break;
            case 'disconnected':
                this.statusText.textContent = 'Disconnected';
                break;
            case 'connecting':
                this.statusText.textContent = 'Connecting...';
                break;
            case 'idle':
                this.statusText.textContent = 'Idle';
                break;
        }

        this.statusChip.title = this.statusText.textContent;
    }
    
    addSystemMessage(text, type = 'info') {
        const div = document.createElement('div');
        div.className = `system-message ${type}`;
        div.setAttribute('role', 'status');
        div.textContent = text;
        this.chat.appendChild(div);
        
        // Auto-remove after 8 seconds
        setTimeout(() => {
            if (div.parentNode) {
                div.remove();
            }
        }, 8000);
    }
    
    showError(message) {
        this.addSystemMessage(message, 'error');
    }
    
    playNotificationSound(force = false) {
        if (!this.audioContext) return;
        if (this.isMuted && !force) return;
        
        try {
            // Resume context if suspended (mobile requirement)
            if (this.audioContext.state === 'suspended') {
                this.audioContext.resume();
            }
            
            // Simple beep sound
            const oscillator = this.audioContext.createOscillator();
            const gainNode = this.audioContext.createGain();
            
            oscillator.connect(gainNode);
            gainNode.connect(this.audioContext.destination);
            
            oscillator.frequency.value = 880;
            oscillator.type = 'sine';
            
            gainNode.gain.setValueAtTime(0.3, this.audioContext.currentTime);
            gainNode.gain.exponentialRampToValueAtTime(0.01, this.audioContext.currentTime + 0.2);
            
            oscillator.start(this.audioContext.currentTime);
            oscillator.stop(this.audioContext.currentTime + 0.2);
        } catch (e) {
            console.warn('Failed to play notification sound:', e);
        }
    }
    
    parseTimestamp(ts) {
        // Validate timestamp - if less than 1e12, assume seconds and convert to ms
        const num = parseInt(ts);
        return num < 1e12 ? num * 1000 : num;
    }
    
    /// Escapes text for interpolation into an innerHTML string.
    ///
    /// Only the quoted-reply block needs this, because it is the one place the
    /// client builds markup as a string. Everywhere else sets `textContent`,
    /// which does not interpret markup at all — passing escaped text to
    /// `textContent` was double-escaping it, so an animal name or reply preview
    /// containing `&` or `<` displayed as `&amp;` or `&lt;`.
    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }
    
    formatTimestamp(ms) {
        const date = new Date(ms);
        return date.toLocaleTimeString([], { 
            hour: '2-digit', 
            minute: '2-digit',
            hour12: true 
        });
    }
    
    cleanup() {
        // Clear all timers and intervals
        if (this.cleanupIntervalId) {
            clearInterval(this.cleanupIntervalId);
            this.cleanupIntervalId = null;
        }
        if (this.typingTimeout) {
            clearTimeout(this.typingTimeout);
            this.typingTimeout = null;
        }
        if (this.scrollTimeoutId) {
            clearTimeout(this.scrollTimeoutId);
            this.scrollTimeoutId = null;
        }
        // Cancel a pending auto-scroll frame
        if (this.autoScrollFrameId) {
            cancelAnimationFrame(this.autoScrollFrameId);
            this.autoScrollFrameId = null;
        }
        if (this.historyLoadTimeoutId) {
            clearTimeout(this.historyLoadTimeoutId);
            this.historyLoadTimeoutId = null;
        }
        // Clear heartbeat interval
        if (this.heartbeatIntervalId) {
            clearInterval(this.heartbeatIntervalId);
            this.heartbeatIntervalId = null;
        }
        // Close WebSocket if open
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this.ws.close();
        }
    }
    
    /// A room you spawned and `main` fail differently, so this watches
    /// different clocks for each (§7.4): `main` fades on room-wide silence
    /// whether or not anyone stays connected, so `idleSeconds` — time since
    /// anyone last spoke — is the honest measure there. A spawned room cannot
    /// be deleted while anyone is connected to it (constraint #3), so the
    /// honest measure for a present viewer is their *own* idle time, which is
    /// what actually gets them disconnected.
    updateHeartbeat() {
        const now = Date.now();
        const isMain = this.roomName === MAIN_ROOM_NAME;
        const idleSeconds = isMain
            ? Math.floor((now - this.lastActivityTime) / 1000)
            : Math.floor((now - this.myLastSentTime) / 1000);
        const gracePeriod = isMain ? MAIN_ROOM_FADE_SECONDS : IDLE_EVICTION_SECONDS;
        const aliveMinutes = Math.floor((now - this.roomStartTime) / 60000);

        // Update lifespan display
        if (aliveMinutes < 60) {
            this.roomLifespan.textContent = `${aliveMinutes}m alive`;
        } else {
            const hours = Math.floor(aliveMinutes / 60);
            const mins = aliveMinutes % 60;
            this.roomLifespan.textContent = `${hours}h ${mins}m alive`;
        }

        // Update heartbeat state based on activity, as fractions of the real
        // grace period — not independently invented numbers (§7.3).
        this.roomHeartbeat.classList.remove('active', 'warning', 'critical');
        this.chat.classList.remove('room-fading');

        if (idleSeconds < 60) {
            // Active: someone spoke — or, in a spawned room, *you* spoke — in
            // the last minute.
            this.roomHeartbeat.classList.add('active');
        } else if (idleSeconds < gracePeriod * 0.7) {
            // Normal: quiet, but most of the grace period remains.
        } else if (idleSeconds < gracePeriod * 0.9) {
            // Warning: the last 30% of the grace period.
            this.roomHeartbeat.classList.add('warning');
            if (!this.fadeWarningShown) {
                this.fadeWarningShown = true;
                this.showFadeWarning(isMain);
            }
        } else {
            // Critical: the last 10%. `main`'s history is genuinely about to
            // trim; a spawned room's *you* are genuinely about to be moved
            // along for idling — the room itself is not at risk while you're
            // reading this.
            this.roomHeartbeat.classList.add('critical');
            if (isMain) this.chat.classList.add('room-fading');
        }
    }

    /// `isMain` picks which of two true things to say: `main`'s memory is
    /// thinning, or you personally are about to be moved along for having
    /// gone quiet. Neither is "the room is about to disappear" — for a
    /// spawned room, that cannot happen while you're still connected to it
    /// (constraint #3), so saying so would be exactly the lie §7.3 exists to
    /// rule out, just relocated rather than fixed.
    showFadeWarning(isMain) {
        const existing = document.querySelector('.fade-warning');
        if (existing) existing.remove();

        const warning = document.createElement('div');
        warning.className = 'fade-warning';
        const message = isMain
            ? "main's older messages are fading... say something to keep them!"
            : "you've gone quiet... say something or you'll be moved along";
        warning.innerHTML = `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-hourglass"/></svg>${message}`;
        document.body.appendChild(warning);

        setTimeout(() => {
            if (warning.parentNode) warning.remove();
        }, 5000);
    }
}

/// How long a navigation click silences the next one, across page loads.
const NAV_COOLDOWN_MS = 800;

/// The sessionStorage key holding when the cooldown started by the last
/// navigating click expires.
///
/// `sessionStorage`, not an in-memory flag: an in-memory flag cannot survive
/// the very reload it exists to guard against. `.nav-back` and `exploreLink`
/// are normal navigations, not client-side routes — each click that actually
/// navigates hands off to a brand-new page running a brand-new copy of this
/// script, with its own fresh state. A flag reset at every page load has
/// already forgotten the previous click by the time the click it needs to
/// suppress happens: a human mashing the link is clicking a freshly loaded
/// page's link each time, and every one of those loads sees a "first" click.
/// `sessionStorage` persists across reloads in the same tab and clears when
/// the tab closes, which is the lifetime this actually needs.
///
/// Found by reproducing in a real browser (Playwright, 15 scripted clicks on
/// the rendered link): an earlier, in-memory-flag version of this guard
/// blocked a double-click landing before the first navigation started, but
/// did nothing against repeated clicks each landing on its own freshly
/// reloaded page — which is what "spamming the button" actually is, and
/// which still flooded the room-join rate limit exactly as before.
const NAV_COOLDOWN_KEY = 'navCooldownUntil';

function withinNavigationCooldown() {
    const until = Number(sessionStorage.getItem(NAV_COOLDOWN_KEY) || 0);
    return Date.now() < until;
}

function startNavigationCooldown() {
    sessionStorage.setItem(NAV_COOLDOWN_KEY, String(Date.now() + NAV_COOLDOWN_MS));
}

/// Lets a plain `<a href>` navigation fire once per cooldown window, not once
/// per click.
function debounceNavigation(selector) {
    const link = document.querySelector(selector);
    if (!link) return;
    link.addEventListener('click', (event) => {
        if (withinNavigationCooldown()) {
            event.preventDefault();
            return;
        }
        startNavigationCooldown();
    });
}

// Initialize app when DOM is ready
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
        new ChatApp();
        debounceNavigation('.nav-back');
    });
} else {
    new ChatApp();
    debounceNavigation('.nav-back');
}
