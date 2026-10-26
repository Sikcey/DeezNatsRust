DeezNats Connect is a native Windows client for the existing Jam host. It follows the webpage's header, page tabs, cards, bottom player, collection modal, lyrics view, and appearance settings. It uses Direct2D/DirectWrite and native PCM audio; it does not embed a browser or include a Deezer engine.

Run `DeezNatsConnect.exe`, then enter the host URL, for example `http://your-name.duckdns.org:3131`. The music PC must keep DeezNats running with Connect enabled. Friends do not need a Deezer account. Change the address under **Settings > Host address > Connect**, or use F5 / **Connect to host** in the standard Windows system menu.

Use **Listen live** to start audio on this device. Volume is local; playback, queue, search, Flow, shuffle, repeat, and autoplay operate on the shared host. Appearance and English/Turkish language are saved separately for each client. Escape closes a collection or lyrics; Space toggles playback when a text field is not focused. Upcoming queue rows can be dragged to reorder.

Queue opens by default; playback controls stay in the bottom bar. New profiles default to Tokyo Night, opacity 40, and blur 20. Existing appearance choices are preserved.

Settings supports temporary host profiles with custom nicknames. Connect through a profile to label new queue additions with its nickname. Names appear in Connect, the host queue, and the browser queue; an updated host is required. Profiles clear when Connect closes, while attribution stays with tracks already in the host queue.

The client stops animations when minimized or inactive. Catalogue, image, playback-control, state polling, audio, and lyric frosting work run off the window thread. Failed artwork requests retry automatically. Audio buffers and image/text caches are bounded. Album background blur is cached and recalculated only when the cover, blur, or window dimensions change.

Build: `cargo build --release --locked -p deeznats-jam`. The resulting executable needs no companion Deezer DLL or WebView runtime.
