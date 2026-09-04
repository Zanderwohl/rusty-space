"""Union of the user's save (authoritative for identity/mass/radius/tags/rotation)
and the extra bodies added from JPL, keyed by the save's id convention."""
import json
save = json.load(open("user_save.json"))
resolved = json.load(open("resolved.json"))

# my slug -> the id the save uses
ALIAS = {
 "sol":"Sol","mercury":"Mercury","venus":"Venus","earth":"Earth","mars":"Mars",
 "Jupiter":"Jupiter","saturn":"Saturn","Uranus":"Uranus","Neptune":"Neptune",
 "luna":"Luna","phobos":"Phobos","deimos":"Deimos","io":"Io","europa":"Europa",
 "ganymede":"Ganymede","callisto":"Callisto","amalthea":"Amalthea","himalia":"Himalia",
 "elara":"Elara","pasiphae":"Pasiphae","sinope":"Sinope","lysithea":"Lysithea",
 "carme":"Carme","ananke":"Ananke","leda":"Leda","thebe":"Thebe","adrastea":"Adrastea",
 "metis":"Metis","mimas":"Mimas","enceladus":"Enceladus","tethys":"Tethys","dione":"Dione",
 "rhea":"Rhea","titan":"Titan","hyperion":"Hyperion","iapetus":"Iapetus","phoebe":"Phoebe",
 "janus":"Janus","epimetheus":"Epimetheus","helene":"Helene","atlas":"Atlas",
 "prometheus":"Prometheus","pandora":"Pandora","pan":"Pan","pluto":"Pluto","charon":"Charon",
 "nix":"Nix","hydra":"Hydra","kerberos":"Kerberos","styx":"Styx","1-ceres":"1-Ceres",
 "4-vesta":"4-Vesta","eris":"Eris","Sedna":"Sedna","dysnomia":"Dysnomia",
}
# bodies I added that the save has no entry for; ids follow the save's convention
EXTRA_IDS = {
 "ariel":"Ariel","umbriel":"Umbriel","titania":"Titania","oberon":"Oberon",
 "miranda":"Miranda","puck":"Puck","triton":"Triton","nereid":"Nereid",
 "despina":"Despina","larissa":"Larissa","proteus":"Proteus",
 "2-pallas":"2-Pallas","3-juno":"3-Juno","10-hygiea":"10-Hygiea","15-eunomia":"15-Eunomia",
 "16-psyche":"16-Psyche","21-lutetia":"21-Lutetia","52-europa":"52-Europa","87-sylvia":"87-Sylvia",
 "216-kleopatra":"216-Kleopatra","243-ida":"243-Ida","253-mathilde":"253-Mathilde",
 "433-eros":"433-Eros","511-davida":"511-Davida","704-interamnia":"704-Interamnia",
 "951-gaspra":"951-Gaspra","1566-icarus":"1566-Icarus","1620-geographos":"1620-Geographos",
 "2867-steins":"2867-Steins","3200-phaethon":"3200-Phaethon","4179-toutatis":"4179-Toutatis",
 "25143-itokawa":"25143-Itokawa","99942-apophis":"99942-Apophis","101955-bennu":"101955-Bennu",
 "162173-ryugu":"162173-Ryugu","136472-makemake":"Makemake","136108-haumea":"Haumea",
 "50000-quaoar":"Quaoar","90482-orcus":"Orcus","225088-gonggong":"Gonggong",
 "28978-ixion":"Ixion","20000-varuna":"Varuna","486958-arrokoth":"Arrokoth","38628-huya":"Huya",
 "174567-varda":"Varda","120347-salacia":"Salacia","2060-chiron":"2060-Chiron",
 "5145-pholus":"5145-Pholus","10199-chariklo":"10199-Chariklo","1p-halley":"1P-Halley",
 "2p-encke":"2P-Encke","9p-tempel1":"9P-Tempel-1","19p-borrelly":"19P-Borrelly",
 "67p-cg":"67P-Churyumov-Gerasimenko","81p-wild2":"81P-Wild-2","103p-hartley2":"103P-Hartley-2",
 "c1995o1-halebopp":"C-1995-O1-Hale-Bopp","1i-oumuamua":"1I-Oumuamua","2i-borisov":"2I-Borisov",
}
PRIMARY_OF_EXTRA = {  # in save-id terms
 "Ariel":"Uranus","Umbriel":"Uranus","Titania":"Uranus","Oberon":"Uranus","Miranda":"Uranus",
 "Puck":"Uranus","Triton":"Neptune","Nereid":"Neptune","Despina":"Neptune","Larissa":"Neptune",
 "Proteus":"Neptune",
}
def primary_of(save_id, my_slug):
    if save_id in save and save[save_id].get("primary"): return save[save_id]["primary"]
    return PRIMARY_OF_EXTRA.get(save_id, "Sol")

if __name__ == "__main__":
    print(f"save bodies {len(save)}, resolved {sum(1 for v in resolved.values() if 'error' not in v)}")
    print(f"extras {len(EXTRA_IDS)}  -> union {len(save) + len(EXTRA_IDS)}")
