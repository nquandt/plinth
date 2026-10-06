import { app, Screen, Section, Image, Icon } from "plinth:ui";

function Gallery() {
  return (
    <Screen title="Gallery">
      <Section title="Photos">
        <Image src="mountain.png" alt="A mountain range at dusk" aspect="square" />
        <Image src="banner.png" alt="A wide green banner" aspect="wide" />
        <Image src="badge.png" alt="A red badge" aspect="tall" />
      </Section>
      <Section title="Icons">
        <Icon name="star" tone="muted" />
        <Icon name="heart" tone="danger" label="Favorite" />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "teal",
  screens: {
    gallery: { title: "Gallery", icon: "star", component: Gallery },
  },
});
