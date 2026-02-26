import { useEffect, useRef } from "react";
import * as THREE from "three";

type Bobber = {
  mesh: THREE.Mesh;
  baseY: number;
  speed: number;
  amp: number;
};

export default function GbaScene() {
  const mountRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) {
      return;
    }

    const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
    renderer.setSize(mount.clientWidth, mount.clientHeight);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.fog = new THREE.Fog(0x63a7ff, 10, 30);

    const camera = new THREE.PerspectiveCamera(48, mount.clientWidth / mount.clientHeight, 0.1, 100);
    camera.position.set(0, 4.5, 9.5);
    camera.lookAt(0, 1.2, 0);

    const ambient = new THREE.AmbientLight(0xffffff, 1.2);
    scene.add(ambient);

    const keyLight = new THREE.DirectionalLight(0xfff6cc, 1.3);
    keyLight.position.set(6, 9, 5);
    scene.add(keyLight);

    const rimLight = new THREE.DirectionalLight(0x88cfff, 0.9);
    rimLight.position.set(-6, 4, -4);
    scene.add(rimLight);

    const floorSize = 1024;
    const floorCanvas = document.createElement("canvas");
    floorCanvas.width = floorSize;
    floorCanvas.height = floorSize;
    const ctx = floorCanvas.getContext("2d");
    if (ctx) {
      const tile = 64;
      for (let y = 0; y < floorSize; y += tile) {
        for (let x = 0; x < floorSize; x += tile) {
          const odd = (x / tile + y / tile) % 2 === 0;
          ctx.fillStyle = odd ? "#8be2a8" : "#6bcf90";
          ctx.fillRect(x, y, tile, tile);
        }
      }
      ctx.strokeStyle = "rgba(255,255,255,0.14)";
      for (let x = 0; x <= floorSize; x += tile) {
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, floorSize);
        ctx.stroke();
      }
      for (let y = 0; y <= floorSize; y += tile) {
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(floorSize, y);
        ctx.stroke();
      }
    }

    const floorTexture = new THREE.CanvasTexture(floorCanvas);
    floorTexture.magFilter = THREE.NearestFilter;
    floorTexture.minFilter = THREE.NearestFilter;
    floorTexture.wrapS = THREE.RepeatWrapping;
    floorTexture.wrapT = THREE.RepeatWrapping;
    floorTexture.repeat.set(8, 8);

    const floor = new THREE.Mesh(
      new THREE.PlaneGeometry(40, 40),
      new THREE.MeshToonMaterial({ map: floorTexture, color: 0xb0ffd8 })
    );
    floor.rotation.x = -Math.PI / 2;
    floor.position.y = -0.8;
    scene.add(floor);

    const spriteGroup = new THREE.Group();
    scene.add(spriteGroup);

    const palette = [0xff8bd1, 0xffb86a, 0x8df3ff, 0xc0ff72, 0xfff38e];
    const bobbers: Bobber[] = [];
    for (let i = 0; i < 12; i += 1) {
      const shape = i % 3;
      const geometry =
        shape === 0
          ? new THREE.IcosahedronGeometry(0.38 + Math.random() * 0.2, 0)
          : shape === 1
            ? new THREE.BoxGeometry(0.56, 0.56, 0.56)
            : new THREE.CapsuleGeometry(0.26, 0.4, 4, 8);
      const material = new THREE.MeshToonMaterial({
        color: palette[i % palette.length],
        emissive: 0x111111,
      });
      const mesh = new THREE.Mesh(geometry, material);
      const angle = (i / 12) * Math.PI * 2;
      const radius = 3.2 + Math.random() * 2.8;
      const y = 0.4 + Math.random() * 2.2;
      mesh.position.set(Math.cos(angle) * radius, y, Math.sin(angle) * radius - 1.4);
      mesh.rotation.set(Math.random(), Math.random() * Math.PI, Math.random());
      spriteGroup.add(mesh);
      bobbers.push({
        mesh,
        baseY: y,
        speed: 0.7 + Math.random() * 1.7,
        amp: 0.06 + Math.random() * 0.18,
      });
    }

    const starGeom = new THREE.BufferGeometry();
    const starCount = 300;
    const starPos = new Float32Array(starCount * 3);
    for (let i = 0; i < starCount; i += 1) {
      const i3 = i * 3;
      starPos[i3] = (Math.random() - 0.5) * 50;
      starPos[i3 + 1] = Math.random() * 22 + 2;
      starPos[i3 + 2] = (Math.random() - 0.5) * 50 - 8;
    }
    starGeom.setAttribute("position", new THREE.BufferAttribute(starPos, 3));
    const stars = new THREE.Points(
      starGeom,
      new THREE.PointsMaterial({ color: 0xffffff, size: 0.08, transparent: true, opacity: 0.8 })
    );
    scene.add(stars);

    const clock = new THREE.Clock();
    let raf = 0;
    const animate = () => {
      const t = clock.getElapsedTime();
      spriteGroup.rotation.y = t * 0.14;
      for (const bob of bobbers) {
        bob.mesh.position.y = bob.baseY + Math.sin(t * bob.speed) * bob.amp;
        bob.mesh.rotation.x += 0.002;
        bob.mesh.rotation.y += 0.0032;
      }
      stars.rotation.y = t * 0.015;
      stars.rotation.x = Math.sin(t * 0.1) * 0.06;
      renderer.render(scene, camera);
      raf = window.requestAnimationFrame(animate);
    };
    animate();

    const handleResize = () => {
      if (!mount) {
        return;
      }
      const w = mount.clientWidth;
      const h = mount.clientHeight;
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
    };
    window.addEventListener("resize", handleResize);

    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", handleResize);
      starGeom.dispose();
      for (const bob of bobbers) {
        bob.mesh.geometry.dispose();
        (bob.mesh.material as THREE.Material).dispose();
      }
      floor.geometry.dispose();
      (floor.material as THREE.Material).dispose();
      floorTexture.dispose();
      renderer.dispose();
      if (renderer.domElement.parentElement === mount) {
        mount.removeChild(renderer.domElement);
      }
    };
  }, []);

  return <div className="gba-scene" ref={mountRef} aria-hidden="true" />;
}
